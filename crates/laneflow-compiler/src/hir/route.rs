//! 路线领域 HIR：静态路线、边序列与预编译路口控制出现项的记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{EntityKind, FieldTag, StaticRouteId};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::TypedAstDeclaration;
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    CanonicalDeclarationSource, HirJunctionInternalEdge, HirLaneEdge, HirLaneEdgeKey,
    HirLaneEdgeReference, HirLaneEdgeTag, HirManeuverGate, HirManeuverGateKey, HirManeuverPath,
    HirManeuverPathEdge, HirManeuverPathGate, HirManeuverPathKey, HirManeuverPathWaitingZone,
    HirModuleKey, HirStaticRouteTag, HirStopLine, HirWaitingZone, HirWaitingZoneKey, RouteCounts,
    SymbolTable, arena_overflow, count_to_usize, declaration_header, derive_identity,
    resolve_reference,
};

/// 静态路线有序边序列中的一次出现；同一 `LaneEdge` 可以出现多次。
#[derive(Debug, PartialEq)]
pub(crate) struct HirStaticRouteEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

/// 静态路线相邻边转换上预编译的可选机动门。
#[derive(Debug, PartialEq)]
pub(crate) struct HirStaticRouteTransition {
    pub(crate) maneuver_gate: Option<HirManeuverGateKey>,
}

/// 一条完整机动路径在静态路线中的一次匹配。
#[derive(Debug, PartialEq)]
pub(crate) struct HirManeuverOccurrence {
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) exit_route_edge_index: u32,
    pub(crate) gate_occurrences: TableRange<HirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<HirWaitingZoneOccurrence>,
}

/// 一个 `ManeuverGate` 在某次路线机动中的预编译出现项。
#[derive(Debug, PartialEq)]
pub(crate) struct HirGateOccurrence {
    pub(crate) maneuver_gate: HirManeuverGateKey,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) from_route_edge_index: u32,
    pub(crate) next_gate_occurrence_index: Option<u32>,
    pub(crate) next_boundary_route_edge_index: u32,
    pub(crate) waiting_zone_occurrence_index: Option<u32>,
}

/// 一个 `WaitingZone` 在某次路线机动中的预编译出现项。
#[derive(Debug, PartialEq)]
pub(crate) struct HirWaitingZoneOccurrence {
    pub(crate) waiting_zone: HirWaitingZoneKey,
    pub(crate) maneuver_occurrence_index: u32,
    pub(crate) entry_gate_occurrence_index: u32,
    pub(crate) release_gate_occurrence_index: u32,
    pub(crate) entry_route_edge_index: u32,
    pub(crate) release_route_edge_index: u32,
}

/// 已解析边序列并闭合全部路口控制出现项的静态路线。
#[derive(Debug, PartialEq)]
pub(crate) struct HirStaticRoute {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: StaticRouteId,
    pub(crate) edges: TableRange<HirStaticRouteEdge>,
    pub(crate) transitions: TableRange<HirStaticRouteTransition>,
    pub(crate) maneuver_occurrences: TableRange<HirManeuverOccurrence>,
    pub(crate) gate_occurrences: TableRange<HirGateOccurrence>,
    pub(crate) waiting_zone_occurrences: TableRange<HirWaitingZoneOccurrence>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct RouteHir {
    pub(crate) static_routes: Box<[HirStaticRoute]>,
    pub(crate) static_route_edges: Box<[HirStaticRouteEdge]>,
    pub(crate) static_route_transitions: Box<[HirStaticRouteTransition]>,
    pub(crate) maneuver_occurrences: Box<[HirManeuverOccurrence]>,
    pub(crate) gate_occurrences: Box<[HirGateOccurrence]>,
    pub(crate) waiting_zone_occurrences: Box<[HirWaitingZoneOccurrence]>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn build_route_hir(
    unit: &CompilationUnit,
    counts: &RouteCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    maneuver_paths: &[HirManeuverPath],
    maneuver_path_edges: &[HirManeuverPathEdge],
    junction_internal_edges: &[HirJunctionInternalEdge],
    stop_lines: &[HirStopLine],
    maneuver_gates: &[HirManeuverGate],
    waiting_zones: &[HirWaitingZone],
    maneuver_path_gates: &[HirManeuverPathGate],
    maneuver_path_waiting_zones: &[HirManeuverPathWaitingZone],
    identities: &mut IdentityRegistry,
) -> Result<RouteHir, DiagnosticBundle> {
    if counts.static_routes == 0 {
        return Ok(RouteHir::default());
    }

    // 候选表按前两条边建立连续排序索引。路线扫描只做二分分段和切片遍历，既不依赖
    // HashMap 迭代顺序，也不会为每条路线重新建立路径查找表。
    let mut entry_candidates = Vec::with_capacity(maneuver_paths.len());
    for (index, path) in maneuver_paths.iter().enumerate() {
        let path_key = HirManeuverPathKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let edges = &maneuver_path_edges[path.edges.as_usize_range()];
        debug_assert!(
            edges.len() >= 2,
            "validated ManeuverPath must have boundaries"
        );
        entry_candidates.push((edges[0].target, edges[1].target, path_key));
    }
    entry_candidates.sort_unstable_by(|left, right| {
        (
            left.0.raw(),
            left.1.raw(),
            maneuver_paths[left.2.index()].stable_id,
        )
            .cmp(&(
                right.0.raw(),
                right.1.raw(),
                maneuver_paths[right.2.index()].stable_id,
            ))
    });

    // 角色索引把路线边界检查和最终覆盖检查降为 O(route edges)。每个内部边槽只保留
    // 路口 HIR 已按 StableId 选出的规范代表 claim；它不表示该边只能被一条路径使用。
    let mut internal_owner = vec![None; lane_edges.len()];
    for claim in junction_internal_edges {
        internal_owner[claim.edge.index()] = Some(claim.source_path);
    }
    let mut stop_line_by_edge = vec![None; lane_edges.len()];
    for (index, stop_line) in stop_lines.iter().enumerate() {
        let slot = &mut stop_line_by_edge[stop_line.lane_edge.index()];
        if slot.is_none_or(|existing: usize| stop_lines[existing].stable_id > stop_line.stable_id) {
            *slot = Some(index);
        }
    }

    let route_capacity = count_to_usize(counts.static_routes, &unit.limits)?;
    let edge_capacity = count_to_usize(counts.route_edges, &unit.limits)?;
    let transition_capacity = count_to_usize(counts.route_transitions, &unit.limits)?;
    let mut routes = TypedArena::<HirStaticRouteTag, HirStaticRoute>::with_capacity(route_capacity);
    let mut sources = Vec::with_capacity(route_capacity);

    // 先按模块规范顺序和 stable key 登记路线身份，使声明物理顺序不影响路线 ordinal。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::StaticRoute(_)).then_some(index)
            })
            .collect::<Vec<_>>();
        declaration_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::StaticRoute(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("route source filter admitted unrelated declaration")
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(FieldTag::RouteKey, source.header.stable_key.as_bytes()),
            ];
            let stable_id = StaticRouteId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::StaticRoute,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let route_key = routes
                .push(HirStaticRoute {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    edges: TableRange::empty(),
                    transitions: TableRange::empty(),
                    maneuver_occurrences: TableRange::empty(),
                    gate_occurrences: TableRange::empty(),
                    waiting_zone_occurrences: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                declaration_index: u32::try_from(declaration_index).map_err(|_| {
                    arena_overflow(
                        ArenaKeyOverflow,
                        &unit.limits,
                        Some(source.header.span.clone()),
                    )
                })?,
                hir_key: route_key,
            });
        }
    }

    let mut route_edges = Vec::with_capacity(edge_capacity);
    let mut route_transitions = Vec::with_capacity(transition_capacity);
    let mut maneuver_occurrences = Vec::with_capacity(edge_capacity);
    let mut gate_occurrences = Vec::with_capacity(edge_capacity);
    let mut waiting_zone_occurrences = Vec::with_capacity(edge_capacity);
    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));

    for location in sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::StaticRoute(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical StaticRoute source changed kind")
        };
        let route_index = |index: usize| {
            u32::try_from(index).map_err(|_| {
                arena_overflow(
                    ArenaKeyOverflow,
                    &unit.limits,
                    Some(source.header.span.clone()),
                )
            })
        };
        let mut resolved_edges = Vec::with_capacity(source.edge_sequence.len());
        let mut route_has_error = false;
        for reference in &source.edge_sequence {
            if let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::StaticRoute,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            ) {
                resolved_edges.push(HirStaticRouteEdge {
                    target,
                    source_span: reference.span.clone(),
                });
            } else {
                route_has_error = true;
            }
        }
        if route_has_error {
            continue;
        }
        debug_assert!(!resolved_edges.is_empty(), "frontend rejects empty routes");

        for (index, pair) in resolved_edges.windows(2).enumerate() {
            let successors =
                &lane_edge_references[lane_edges.get(pair[0].target).successors.as_usize_range()];
            let has_explicit_successor = successors
                .iter()
                .any(|successor| successor.target == pair[1].target);
            // Junction-internal transitions intentionally use ManeuverPath as their sole topology
            // authority. Exact full-path matching and internal coverage below reject wrong exits or
            // partial paths, so only a pair wholly outside internal ownership needs a successor.
            let touches_internal_edge = internal_owner[pair[0].target.index()].is_some()
                || internal_owner[pair[1].target.index()].is_some();
            if !has_explicit_successor && !touches_internal_edge {
                let mut diagnostic = Diagnostic::disconnected_static_route_edge(
                    &source.header.stable_key,
                    &lane_edges.get(pair[0].target).stable_key,
                    &lane_edges.get(pair[1].target).stable_key,
                    route_index(index.saturating_add(1))?,
                    pair[1].source_span.clone(),
                    pair[0].source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
            }
        }
        let first_edge = resolved_edges[0].target;
        if let Some(path_key) = internal_owner[first_edge.index()] {
            let mut diagnostic = Diagnostic::static_route_starts_inside_junction(
                &source.header.stable_key,
                &lane_edges.get(first_edge).stable_key,
                resolved_edges[0].source_span.clone(),
                maneuver_paths[path_key.index()].source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            route_has_error = true;
        }
        let last_index = resolved_edges.len().saturating_sub(1);
        let last_edge = resolved_edges[last_index].target;
        if let Some(path_key) = internal_owner[last_edge.index()] {
            let mut diagnostic = Diagnostic::static_route_ends_inside_junction(
                &source.header.stable_key,
                &lane_edges.get(last_edge).stable_key,
                resolved_edges[last_index].source_span.clone(),
                maneuver_paths[path_key.index()].source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            route_has_error = true;
        }
        if let Some(stop_index) = stop_line_by_edge[last_edge.index()] {
            let stop_line = &stop_lines[stop_index];
            let mut diagnostic = Diagnostic::static_route_terminates_at_stop_line(
                &source.header.stable_key,
                &lane_edges.get(last_edge).stable_key,
                &stop_line.stable_key,
                resolved_edges[last_index].source_span.clone(),
                stop_line.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            route_has_error = true;
        }
        if route_has_error {
            continue;
        }

        let mut local_transitions = (0..resolved_edges.len().saturating_sub(1))
            .map(|_| HirStaticRouteTransition {
                maneuver_gate: None,
            })
            .collect::<Vec<_>>();
        let mut local_maneuvers = Vec::with_capacity(resolved_edges.len());
        let mut local_gates = Vec::with_capacity(resolved_edges.len());
        let mut local_waiting = Vec::with_capacity(resolved_edges.len());
        let mut internal_coverage: Vec<Option<HirManeuverPathKey>> =
            vec![None; resolved_edges.len()];

        for entry_index in 0..resolved_edges.len().saturating_sub(1) {
            let pair = (
                resolved_edges[entry_index].target,
                resolved_edges[entry_index + 1].target,
            );
            let candidate_start = entry_candidates.partition_point(|candidate| {
                (candidate.0.raw(), candidate.1.raw()) < (pair.0.raw(), pair.1.raw())
            });
            let candidate_end = entry_candidates.partition_point(|candidate| {
                (candidate.0.raw(), candidate.1.raw()) <= (pair.0.raw(), pair.1.raw())
            });
            if candidate_start == candidate_end {
                continue;
            }
            let candidates = &entry_candidates[candidate_start..candidate_end];
            let mut full_matches = candidates.iter().filter_map(|candidate| {
                let path = &maneuver_paths[candidate.2.index()];
                let path_edges = &maneuver_path_edges[path.edges.as_usize_range()];
                (resolved_edges.len().saturating_sub(entry_index) >= path_edges.len()
                    && resolved_edges[entry_index..entry_index + path_edges.len()]
                        .iter()
                        .map(|edge| edge.target)
                        .eq(path_edges.iter().map(|edge| edge.target)))
                .then_some(candidate.2)
            });
            let Some(path_key) = full_matches.next() else {
                let candidate = candidates[0].2;
                let mut diagnostic = Diagnostic::static_route_maneuver_no_full_match(
                    &source.header.stable_key,
                    route_index(entry_index)?,
                    &lane_edges.get(pair.0).stable_key,
                    &lane_edges.get(pair.1).stable_key,
                    resolved_edges[entry_index + 1].source_span.clone(),
                    maneuver_paths[candidate.index()].source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
                continue;
            };
            if let Some(second_path_key) = full_matches.next() {
                let first = &maneuver_paths[path_key.index()];
                let second = &maneuver_paths[second_path_key.index()];
                let mut diagnostic = Diagnostic::static_route_maneuver_multiple_full_matches(
                    &source.header.stable_key,
                    route_index(entry_index)?,
                    &first.stable_key,
                    &second.stable_key,
                    resolved_edges[entry_index].source_span.clone(),
                    first.source_span.clone(),
                    second.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
                continue;
            }

            let path = &maneuver_paths[path_key.index()];
            let path_edge_count = path.edges.as_usize_range().len();
            let exit_index = entry_index + path_edge_count.saturating_sub(1);
            for route_edge_index in entry_index + 1..exit_index {
                if let Some(first_path_key) = internal_coverage[route_edge_index] {
                    let first = &maneuver_paths[first_path_key.index()];
                    let mut diagnostic = Diagnostic::static_route_maneuver_internal_overlap(
                        &source.header.stable_key,
                        route_index(route_edge_index)?,
                        &lane_edges
                            .get(resolved_edges[route_edge_index].target)
                            .stable_key,
                        &first.stable_key,
                        &path.stable_key,
                        resolved_edges[route_edge_index].source_span.clone(),
                        first.source_span.clone(),
                        path.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                    route_has_error = true;
                } else {
                    internal_coverage[route_edge_index] = Some(path_key);
                }
            }

            let maneuver_index = local_maneuvers.len();
            let local_gate_start = local_gates.len();
            for member in &maneuver_path_gates[path.maneuver_gates.as_usize_range()] {
                let gate = &maneuver_gates[member.maneuver_gate.index()];
                let from_route_edge_index = entry_index + gate.transition_index as usize;
                local_transitions[from_route_edge_index].maneuver_gate = Some(member.maneuver_gate);
                local_gates.push(HirGateOccurrence {
                    maneuver_gate: member.maneuver_gate,
                    maneuver_occurrence_index: route_index(maneuver_index)?,
                    from_route_edge_index: route_index(from_route_edge_index)?,
                    next_gate_occurrence_index: None,
                    next_boundary_route_edge_index: route_index(exit_index)?,
                    waiting_zone_occurrence_index: None,
                });
            }
            let local_gate_end = local_gates.len();
            for gate_index in local_gate_start..local_gate_end {
                if gate_index + 1 < local_gate_end {
                    local_gates[gate_index].next_gate_occurrence_index =
                        Some(route_index(gate_index + 1)?);
                    local_gates[gate_index].next_boundary_route_edge_index =
                        local_gates[gate_index + 1].from_route_edge_index;
                }
            }

            let local_waiting_start = local_waiting.len();
            for member in &maneuver_path_waiting_zones[path.waiting_zones.as_usize_range()] {
                let waiting = &waiting_zones[member.waiting_zone.index()];
                let entry_gate_offset = maneuver_path_gates[path.maneuver_gates.as_usize_range()]
                    .iter()
                    .position(|gate| gate.maneuver_gate == waiting.entry_gate)
                    .expect("validated WaitingZone entry gate belongs to path");
                let release_gate_offset = maneuver_path_gates[path.maneuver_gates.as_usize_range()]
                    .iter()
                    .position(|gate| gate.maneuver_gate == waiting.release_gate)
                    .expect("validated WaitingZone release gate belongs to path");
                let entry_gate_index = local_gate_start + entry_gate_offset;
                let release_gate_index = local_gate_start + release_gate_offset;
                let waiting_index = local_waiting.len();
                local_gates[entry_gate_index].waiting_zone_occurrence_index =
                    Some(route_index(waiting_index)?);
                local_waiting.push(HirWaitingZoneOccurrence {
                    waiting_zone: member.waiting_zone,
                    maneuver_occurrence_index: route_index(maneuver_index)?,
                    entry_gate_occurrence_index: route_index(entry_gate_index)?,
                    release_gate_occurrence_index: route_index(release_gate_index)?,
                    entry_route_edge_index: local_gates[entry_gate_index].from_route_edge_index,
                    release_route_edge_index: local_gates[release_gate_index].from_route_edge_index,
                });
            }
            local_maneuvers.push(HirManeuverOccurrence {
                maneuver_path: path_key,
                entry_route_edge_index: route_index(entry_index)?,
                exit_route_edge_index: route_index(exit_index)?,
                gate_occurrences: TableRange::try_from_usize(
                    gate_occurrences.len() + local_gate_start,
                    local_gate_end.saturating_sub(local_gate_start),
                )
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?,
                waiting_zone_occurrences: TableRange::try_from_usize(
                    waiting_zone_occurrences.len() + local_waiting_start,
                    local_waiting.len().saturating_sub(local_waiting_start),
                )
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?,
            });
        }

        for (route_edge_index, route_edge) in resolved_edges.iter().enumerate() {
            let Some(owner_path) = internal_owner[route_edge.target.index()] else {
                continue;
            };
            if internal_coverage[route_edge_index].is_none() {
                let mut diagnostic = Diagnostic::static_route_internal_edge_uncovered(
                    &source.header.stable_key,
                    route_index(route_edge_index)?,
                    &lane_edges.get(route_edge.target).stable_key,
                    route_edge.source_span.clone(),
                    maneuver_paths[owner_path.index()].source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                route_has_error = true;
            }
        }
        if route_has_error {
            continue;
        }

        let edge_start = route_edges.len();
        let transition_start = route_transitions.len();
        let maneuver_start = maneuver_occurrences.len();
        let gate_start = gate_occurrences.len();
        let waiting_start = waiting_zone_occurrences.len();
        route_edges.extend(resolved_edges);
        route_transitions.extend(local_transitions);
        maneuver_occurrences.extend(local_maneuvers);
        gate_occurrences.extend(local_gates);
        waiting_zone_occurrences.extend(local_waiting);
        let route = routes.get_mut(location.hir_key);
        route.edges = TableRange::try_from_usize(edge_start, route_edges.len() - edge_start)
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        route.transitions = TableRange::try_from_usize(
            transition_start,
            route_transitions.len() - transition_start,
        )
        .map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;
        route.maneuver_occurrences =
            TableRange::try_from_usize(maneuver_start, maneuver_occurrences.len() - maneuver_start)
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
        route.gate_occurrences =
            TableRange::try_from_usize(gate_start, gate_occurrences.len() - gate_start).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;
        route.waiting_zone_occurrences = TableRange::try_from_usize(
            waiting_start,
            waiting_zone_occurrences.len() - waiting_start,
        )
        .map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    Ok(RouteHir {
        static_routes: routes.into_boxed_slice(),
        static_route_edges: route_edges.into_boxed_slice(),
        static_route_transitions: route_transitions.into_boxed_slice(),
        maneuver_occurrences: maneuver_occurrences.into_boxed_slice(),
        gate_occurrences: gate_occurrences.into_boxed_slice(),
        waiting_zone_occurrences: waiting_zone_occurrences.into_boxed_slice(),
    })
}
