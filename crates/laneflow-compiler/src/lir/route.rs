//! 静态路线领域 Canonical LIR 记录。

use laneflow_static_contract::{
    LaneEdgeOrdinal, ManeuverGateOrdinal, ManeuverPathOrdinal, StaticRouteId, StaticRouteOrdinal,
    WaitingZoneOrdinal,
};

use crate::arena::TableRange;

use super::LirIdentityField;

pub(crate) struct LirStaticRouteTransition {
    pub(crate) maneuver_gate: Option<ManeuverGateOrdinal>,
}

pub(crate) struct LirManeuverOccurrence {
    pub(crate) maneuver_path: ManeuverPathOrdinal,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) exit_route_edge_index: u32,
    pub(crate) gate_occurrences: TableRange<LirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<LirWaitingZoneOccurrence>,
}

pub(crate) struct LirGateOccurrence {
    pub(crate) maneuver_gate: ManeuverGateOrdinal,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) from_route_edge_index: u32,
    pub(crate) next_gate_occurrence_index: Option<u32>,
    pub(crate) next_boundary_route_edge_index: u32,
    pub(crate) waiting_zone_occurrence_index: Option<u32>,
}

pub(crate) struct LirWaitingZoneOccurrence {
    pub(crate) waiting_zone: WaitingZoneOrdinal,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) entry_gate_occurrence_index: u32,
    pub(crate) release_gate_occurrence_index: u32,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) release_route_edge_index: u32,
}

pub(crate) struct LirStaticRoute {
    pub(crate) ordinal: StaticRouteOrdinal,
    pub(crate) stable_id: StaticRouteId,
    pub(crate) identity_fields: TableRange<LirIdentityField>,
    pub(crate) edges: TableRange<LaneEdgeOrdinal>,
    pub(crate) transitions: TableRange<LirStaticRouteTransition>,
    pub(crate) maneuver_occurrences: TableRange<LirManeuverOccurrence>,
    pub(crate) gate_occurrences: TableRange<LirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<LirWaitingZoneOccurrence>,
}

/// 从稳定实体反查静态路线出现项；`occurrence_index` 是对应路线内的局部下标。
#[derive(Clone, Copy)]
pub(crate) struct LirRouteOccurrenceRef {
    pub(crate) static_route: StaticRouteOrdinal,
    pub(crate) occurrence_index: u32,
}

use super::{
    FreezeEnv, LirLaneEdge, LirManeuverGate, LirManeuverPath, LirRouteCounts, LirWaitingZone,
    freeze_reverse_occurrences, push_lir_identity, relation_range, table_overflow,
};
use crate::DiagnosticBundle;
use laneflow_static_contract::FieldTag;

pub(super) struct RouteParts {
    pub static_routes: Vec<LirStaticRoute>,
    pub static_route_edges: Vec<LaneEdgeOrdinal>,
    pub static_route_transitions: Vec<LirStaticRouteTransition>,
    pub maneuver_occurrences: Vec<LirManeuverOccurrence>,
    pub gate_occurrences: Vec<LirGateOccurrence>,
    pub waiting_zone_occurrences: Vec<LirWaitingZoneOccurrence>,
    pub lane_edge_route_occurrences: Vec<LirRouteOccurrenceRef>,
    pub maneuver_path_route_occurrences: Vec<LirRouteOccurrenceRef>,
    pub maneuver_gate_route_occurrences: Vec<LirRouteOccurrenceRef>,
    pub waiting_zone_route_occurrences: Vec<LirRouteOccurrenceRef>,
}

pub(super) fn freeze(
    env: &mut FreezeEnv<'_>,
    counts: &LirRouteCounts,
    reverse_occurrence_count: u64,
    lane_edges: &mut [LirLaneEdge],
    maneuver_paths: &mut [LirManeuverPath],
    maneuver_gates: &mut [LirManeuverGate],
    waiting_zones: &mut [LirWaitingZone],
) -> Result<RouteParts, DiagnosticBundle> {
    let mut static_routes = Vec::with_capacity(env.capacity(counts.static_routes)?);
    let mut static_route_edges = Vec::with_capacity(env.capacity(counts.route_edges)?);
    let mut static_route_transitions = Vec::with_capacity(env.capacity(counts.route_transitions)?);
    let mut maneuver_occurrences = Vec::with_capacity(env.capacity(counts.maneuver_occurrences)?);
    let mut gate_occurrences = Vec::with_capacity(env.capacity(counts.gate_occurrences)?);
    let mut waiting_zone_occurrences =
        Vec::with_capacity(env.capacity(counts.waiting_occurrences)?);
    let mut edge_reverse = Vec::with_capacity(env.capacity(counts.route_edges)?);
    let mut path_reverse = Vec::with_capacity(env.capacity(counts.maneuver_occurrences)?);
    let mut gate_reverse = Vec::with_capacity(env.capacity(counts.gate_occurrences)?);
    let mut waiting_reverse = Vec::with_capacity(env.capacity(counts.waiting_occurrences)?);

    for mir_key in env
        .orders
        .static_routes
        .stage_keys_in_lir_order()
        .iter()
        .copied()
    {
        let route = &env.mir.static_routes[mir_key.index()];
        let route_ordinal = env.orders.static_routes.ordinal(mir_key);
        let identity_range = push_lir_identity(
            env.identity_fields,
            env.identity_field_bytes,
            FieldTag::RouteKey,
            &env.mir.modules[route.module.index()].authoring_namespace_id,
            &route.stable_key,
            None,
            env.limits,
            env.primary_span.clone(),
        )?;

        let edge_start = static_route_edges.len();
        for (local_index, edge) in env.mir.static_route_edges[route.edges.as_usize_range()]
            .iter()
            .enumerate()
        {
            let ordinal = env.orders.lane_edges.ordinal(edge.target);
            static_route_edges.push(ordinal);
            edge_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }
        let transition_start = static_route_transitions.len();
        static_route_transitions.extend(
            env.mir.static_route_transitions[route.transitions.as_usize_range()]
                .iter()
                .map(|transition| LirStaticRouteTransition {
                    maneuver_gate: transition
                        .maneuver_gate
                        .map(|key| env.orders.maneuver_gates.ordinal(key)),
                }),
        );

        let gate_start = gate_occurrences.len();
        for (local_index, occurrence) in env.mir.gate_occurrences
            [route.gate_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let ordinal = env.orders.maneuver_gates.ordinal(occurrence.maneuver_gate);
            gate_occurrences.push(LirGateOccurrence {
                maneuver_gate: ordinal,
                maneuver_occurrence_index: occurrence.maneuver_occurrence_index,
                from_route_edge_index: occurrence.from_route_edge_index,
                next_gate_occurrence_index: occurrence.next_gate_occurrence_index,
                next_boundary_route_edge_index: occurrence.next_boundary_route_edge_index,
                waiting_zone_occurrence_index: occurrence.waiting_zone_occurrence_index,
            });
            gate_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }
        let waiting_start = waiting_zone_occurrences.len();
        for (local_index, occurrence) in env.mir.waiting_zone_occurrences
            [route.waiting_zone_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let ordinal = env.orders.waiting_zones.ordinal(occurrence.waiting_zone);
            waiting_zone_occurrences.push(LirWaitingZoneOccurrence {
                waiting_zone: ordinal,
                maneuver_occurrence_index: occurrence.maneuver_occurrence_index,
                entry_gate_occurrence_index: occurrence.entry_gate_occurrence_index,
                release_gate_occurrence_index: occurrence.release_gate_occurrence_index,
                entry_route_edge_index: occurrence.entry_route_edge_index,
                release_route_edge_index: occurrence.release_route_edge_index,
            });
            waiting_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }

        let maneuver_start = maneuver_occurrences.len();
        for (local_index, occurrence) in env.mir.maneuver_occurrences
            [route.maneuver_occurrences.as_usize_range()]
        .iter()
        .enumerate()
        {
            let ordinal = env.orders.maneuver_paths.ordinal(occurrence.maneuver_path);
            let gate_local_start = occurrence
                .gate_occurrences
                .start()
                .saturating_sub(route.gate_occurrences.start());
            let waiting_local_start = occurrence
                .waiting_zone_occurrences
                .start()
                .saturating_sub(route.waiting_zone_occurrences.start());
            maneuver_occurrences.push(LirManeuverOccurrence {
                maneuver_path: ordinal,
                entry_route_edge_index: occurrence.entry_route_edge_index,
                exit_route_edge_index: occurrence.exit_route_edge_index,
                gate_occurrences: TableRange::try_from_usize(
                    gate_start + gate_local_start as usize,
                    occurrence.gate_occurrences.len() as usize,
                )
                .map_err(|overflow| {
                    table_overflow(overflow, env.limits, env.primary_span.clone())
                })?,
                waiting_zone_occurrences: TableRange::try_from_usize(
                    waiting_start + waiting_local_start as usize,
                    occurrence.waiting_zone_occurrences.len() as usize,
                )
                .map_err(|overflow| {
                    table_overflow(overflow, env.limits, env.primary_span.clone())
                })?,
            });
            path_reverse.push((
                ordinal.raw(),
                LirRouteOccurrenceRef {
                    static_route: route_ordinal,
                    occurrence_index: u32::try_from(local_index).unwrap_or(u32::MAX),
                },
            ));
        }

        static_routes.push(LirStaticRoute {
            ordinal: route_ordinal,
            stable_id: route.stable_id,
            identity_fields: identity_range,
            edges: relation_range(
                edge_start,
                static_route_edges.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            transitions: relation_range(
                transition_start,
                static_route_transitions.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            maneuver_occurrences: relation_range(
                maneuver_start,
                maneuver_occurrences.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            gate_occurrences: relation_range(
                gate_start,
                gate_occurrences.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
            waiting_zone_occurrences: relation_range(
                waiting_start,
                waiting_zone_occurrences.len(),
                env.limits,
                env.primary_span.clone(),
            )?,
        });
    }

    let lane_edge_route_occurrences = freeze_reverse_occurrences(
        edge_reverse,
        lane_edges,
        |entity, range| entity.static_route_occurrences = range,
        env.limits,
        env.primary_span.clone(),
    )?;
    let maneuver_path_route_occurrences = freeze_reverse_occurrences(
        path_reverse,
        maneuver_paths,
        |entity, range| entity.static_route_occurrences = range,
        env.limits,
        env.primary_span.clone(),
    )?;
    let maneuver_gate_route_occurrences = freeze_reverse_occurrences(
        gate_reverse,
        maneuver_gates,
        |entity, range| entity.static_route_occurrences = range,
        env.limits,
        env.primary_span.clone(),
    )?;
    let waiting_zone_route_occurrences = freeze_reverse_occurrences(
        waiting_reverse,
        waiting_zones,
        |entity, range| entity.static_route_occurrences = range,
        env.limits,
        env.primary_span.clone(),
    )?;
    debug_assert_eq!(
        lane_edge_route_occurrences.len()
            + maneuver_path_route_occurrences.len()
            + maneuver_gate_route_occurrences.len()
            + waiting_zone_route_occurrences.len(),
        env.capacity(reverse_occurrence_count)?
    );
    Ok(RouteParts {
        static_routes,
        static_route_edges,
        static_route_transitions,
        maneuver_occurrences,
        gate_occurrences,
        waiting_zone_occurrences,
        lane_edge_route_occurrences,
        maneuver_path_route_occurrences,
        maneuver_gate_route_occurrences,
        waiting_zone_route_occurrences,
    })
}
