//! 通行权控制（control）领域 HIR：停止线、机动门与等待区的记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, FieldTag, ManeuverGateId, StableId128, StopLineId, WaitingZoneId,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{TypedAstDeclaration, TypedAstEntityAddress};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{
    CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation,
    WaitingZoneGateRole,
};

use super::{
    CanonicalDeclarationSource, ControlCounts, HirLaneEdge, HirLaneEdgeKey, HirLaneEdgeReference,
    HirLaneEdgeTag, HirManeuverGateKey, HirManeuverGateTag, HirManeuverPath, HirManeuverPathEdge,
    HirManeuverPathKey, HirModuleKey, HirSignalControl, HirStopLineKey, HirStopLineTag,
    HirWaitingZoneKey, HirWaitingZoneTag, SymbolTable, arena_overflow, count_to_usize,
    declaration_header, derive_identity, resolve_reference,
};

/// 机动路径规范门序列中的一项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirManeuverPathGate {
    pub(crate) maneuver_gate: HirManeuverGateKey,
}

/// 机动路径规范等待区序列中的一项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirManeuverPathWaitingZone {
    pub(crate) waiting_zone: HirWaitingZoneKey,
}

/// 停止线到引用它的机动门的反向关系项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirStopLineManeuverGate {
    pub(crate) maneuver_gate: HirManeuverGateKey,
}

/// 已解析边位置并证明至少被一个机动门使用的停止线。
#[derive(Debug, PartialEq)]
pub(crate) struct HirStopLine {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: StopLineId,
    pub(crate) lane_edge: HirLaneEdgeKey,
    pub(crate) maneuver_gates: TableRange<HirStopLineManeuverGate>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到合法路径转换和同边停止线的机动门。
#[derive(Debug, PartialEq)]
pub(crate) struct HirManeuverGate {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    pub(crate) stable_id: ManeuverGateId,
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) transition_index: u32,
    pub(crate) stop_line: HirStopLineKey,
    pub(crate) stop_line_source_location: Option<ResolvedSourceLocation>,
    /// 信号层绑定；`None` 不改变其他通行权层的约束。
    pub(crate) signal_control: HirSignalControl,
    pub(crate) source_span: SourceLocation,
}

/// 已证明门所有权、严格正向区间和同路径内部不重叠的等待区。
#[derive(Debug, PartialEq)]
pub(crate) struct HirWaitingZone {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: WaitingZoneId,
    pub(crate) maneuver_path: HirManeuverPathKey,
    pub(crate) maneuver_path_source_location: Option<ResolvedSourceLocation>,
    pub(crate) entry_gate: HirManeuverGateKey,
    pub(crate) release_gate: HirManeuverGateKey,
    pub(crate) max_occupancy: u32,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct ControlHir {
    pub(crate) stop_lines: Box<[HirStopLine]>,
    pub(crate) maneuver_gates: Box<[HirManeuverGate]>,
    pub(crate) waiting_zones: Box<[HirWaitingZone]>,
    pub(crate) maneuver_path_gates: Box<[HirManeuverPathGate]>,
    pub(crate) maneuver_path_waiting_zones: Box<[HirManeuverPathWaitingZone]>,
    pub(crate) stop_line_maneuver_gates: Box<[HirStopLineManeuverGate]>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn build_control_hir(
    unit: &CompilationUnit,
    counts: &ControlCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    maneuver_paths: &mut [HirManeuverPath],
    maneuver_path_edges: &[HirManeuverPathEdge],
    identities: &mut IdentityRegistry,
) -> Result<ControlHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(ControlHir::default());
    }

    let mut path_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ManeuverPath(_)))
            .count()
    }));
    for (index, path) in maneuver_paths.iter().enumerate() {
        let key = HirManeuverPathKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        path_symbols.insert(path.module, path.source_address.clone(), key);
    }

    let mut stop_lines = TypedArena::<HirStopLineTag, HirStopLine>::with_capacity(count_to_usize(
        counts.stop_lines,
        &unit.limits,
    )?);
    let mut gates = TypedArena::<HirManeuverGateTag, HirManeuverGate>::with_capacity(
        count_to_usize(counts.maneuver_gates, &unit.limits)?,
    );
    let mut waiting_zones = TypedArena::<HirWaitingZoneTag, HirWaitingZone>::with_capacity(
        count_to_usize(counts.waiting_zones, &unit.limits)?,
    );
    let mut stop_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::StopLine(_)))
            .count()
    }));
    let mut gate_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ManeuverGate(_)))
            .count()
    }));
    let mut stop_sources = Vec::with_capacity(count_to_usize(counts.stop_lines, &unit.limits)?);
    let mut gate_sources = Vec::with_capacity(count_to_usize(counts.maneuver_gates, &unit.limits)?);
    let mut waiting_sources =
        Vec::with_capacity(count_to_usize(counts.waiting_zones, &unit.limits)?);

    // 先登记全部控制对象符号，保证声明顺序不影响前向引用；依赖父项的身份先放零值，
    // 只有引用与领域约束全部闭合后才会离开本函数。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(
                    declaration,
                    TypedAstDeclaration::StopLine(_)
                        | TypedAstDeclaration::ManeuverGate(_)
                        | TypedAstDeclaration::WaitingZone(_)
                )
                .then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), &left.source_address)
                .cmp(&(right.entity_kind.code(), &right.source_address))
        });
        for declaration_index in declaration_indices {
            let source_index = u32::try_from(declaration_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[declaration_index] {
                TypedAstDeclaration::StopLine(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::StopLineKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = StopLineId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::StopLine,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = stop_lines
                        .push(HirStopLine {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            lane_edge: HirLaneEdgeKey::from_raw(0),
                            maneuver_gates: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    stop_symbols.insert(module_key, source.header.source_address.clone(), key);
                    stop_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::ManeuverGate(source) => {
                    let key = gates
                        .push(HirManeuverGate {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            source_address: source.header.source_address.clone(),
                            stable_id: ManeuverGateId::from_untyped(StableId128::ZERO),
                            maneuver_path: HirManeuverPathKey::from_raw(0),
                            maneuver_path_source_location: None,
                            transition_index: source.transition_index,
                            stop_line: HirStopLineKey::from_raw(0),
                            stop_line_source_location: None,
                            signal_control: HirSignalControl::None,
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    gate_symbols.insert(module_key, source.header.source_address.clone(), key);
                    gate_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::WaitingZone(source) => {
                    let key = waiting_zones
                        .push(HirWaitingZone {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: WaitingZoneId::from_untyped(StableId128::ZERO),
                            maneuver_path: HirManeuverPathKey::from_raw(0),
                            maneuver_path_source_location: None,
                            entry_gate: HirManeuverGateKey::from_raw(0),
                            release_gate: HirManeuverGateKey::from_raw(0),
                            max_occupancy: source.max_occupancy,
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    waiting_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                _ => unreachable!("control source filter admitted unrelated declaration"),
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    // StopLine 对 LaneEdge 是一对一关系。该表既在解析阶段发现重复所有者，也在后续
    // 覆盖校验中把候选 ManeuverPath 反查到唯一 StopLine，避免按停止线反复扫描路径。
    let mut stop_line_by_edge = vec![None; lane_edges.len()];
    for location in &stop_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::StopLine(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical StopLine source changed kind")
        };
        if let Some(edge) = resolve_reference(
            module_lookup,
            lane_edge_symbols,
            &source.lane_edge,
            EntityKind::StopLine,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) {
            if let Some(first_key) = stop_line_by_edge[edge.index()] {
                let first = stop_lines.get(first_key);
                let duplicate = stop_lines.get(location.hir_key);
                let mut diagnostic = Diagnostic::duplicate_stop_line_edge(
                    &lane_edges.get(edge).stable_key,
                    &first.stable_key,
                    &duplicate.stable_key,
                    duplicate.source_span.clone(),
                    first.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                stop_line_by_edge[edge.index()] = Some(location.hir_key);
            }
            stop_lines.get_mut(location.hir_key).lane_edge = edge;
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut resolved_gate_keys = Vec::with_capacity(gates.len());
    for location in &gate_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::ManeuverGate(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical ManeuverGate source changed kind")
        };
        let path = resolve_reference(
            module_lookup,
            &path_symbols,
            &source.maneuver_path,
            EntityKind::ManeuverGate,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let stop_line = resolve_reference(
            module_lookup,
            &stop_symbols,
            &source.stop_line,
            EntityKind::ManeuverGate,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let (Some(path_key), Some(stop_line_key)) = (path, stop_line) else {
            continue;
        };
        let path = &maneuver_paths[path_key.index()];
        let transition_count = path.edges.len().saturating_sub(1);
        if source.transition_index >= transition_count {
            let mut diagnostic = Diagnostic::maneuver_gate_transition_out_of_range(
                &source.header.stable_key,
                &path.stable_key,
                source.transition_index,
                transition_count,
                source.header.span.clone(),
                path.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            continue;
        }
        let from_edge = maneuver_path_edges[path.edges.as_usize_range()]
            [source.transition_index as usize]
            .target;
        let stop = stop_lines.get(stop_line_key);
        if stop.lane_edge != from_edge {
            let mut diagnostic = Diagnostic::maneuver_gate_stop_line_mismatch(
                &source.header.stable_key,
                &stop.stable_key,
                &lane_edges.get(from_edge).stable_key,
                &lane_edges.get(stop.lane_edge).stable_key,
                source.header.span.clone(),
                stop.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            continue;
        }
        let path_id = path.stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::ManeuverPathStableId, path_id.as_bytes()),
            IdentityFieldInput::new(FieldTag::GateKey, source.header.stable_key.as_bytes()),
        ];
        let stable_id = ManeuverGateId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::ManeuverGate,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let gate = gates.get_mut(location.hir_key);
        gate.stable_id = stable_id;
        gate.maneuver_path = path_key;
        gate.maneuver_path_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.maneuver_path.span,
        )?);
        gate.stop_line = stop_line_key;
        gate.stop_line_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.stop_line.span,
        )?);
        resolved_gate_keys.push(location.hir_key);
    }

    resolved_gate_keys.sort_unstable_by(|left, right| {
        let left = gates.get(*left);
        let right = gates.get(*right);
        (
            left.maneuver_path.raw(),
            left.transition_index,
            left.stable_key.as_bytes(),
        )
            .cmp(&(
                right.maneuver_path.raw(),
                right.transition_index,
                right.stable_key.as_bytes(),
            ))
    });
    for pair in resolved_gate_keys.windows(2) {
        let first = gates.get(pair[0]);
        let duplicate = gates.get(pair[1]);
        if first.maneuver_path == duplicate.maneuver_path
            && first.transition_index == duplicate.transition_index
        {
            let path = &maneuver_paths[first.maneuver_path.index()];
            let mut diagnostic = Diagnostic::duplicate_maneuver_gate_path_transition(
                &path.stable_key,
                first.transition_index,
                &first.stable_key,
                &duplicate.stable_key,
                duplicate.source_span.clone(),
                first.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(duplicate.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut path_gate_counts = vec![0_usize; maneuver_paths.len()];
    let mut stop_gate_counts = vec![0_usize; stop_lines.len()];
    // 使用 u8 标志，使实际容量与上方 scratch 字节预算保持一一对应。
    let mut path_has_entry_gate = vec![0_u8; maneuver_paths.len()];
    let mut stop_has_entry_gate = vec![0_u8; stop_lines.len()];
    for gate_key in &resolved_gate_keys {
        let gate = gates.get(*gate_key);
        path_gate_counts[gate.maneuver_path.index()] =
            path_gate_counts[gate.maneuver_path.index()].saturating_add(1);
        stop_gate_counts[gate.stop_line.index()] =
            stop_gate_counts[gate.stop_line.index()].saturating_add(1);
        if gate.transition_index == 0 {
            path_has_entry_gate[gate.maneuver_path.index()] = 1;
            stop_has_entry_gate[gate.stop_line.index()] = 1;
        }
    }

    // 每个显式 successor 引用是否至少有一条 ManeuverPath 使用该转换；另行记录所有
    // path transition 的起始边，使由 ManeuverPath 独占权威的 junction-internal 转换无需
    // 伪造 successor，也能合法承载 release gate 的 stop line。
    let mut successor_has_path = vec![0_u8; lane_edge_references.len()];
    let mut edge_has_path_transition = vec![0_u8; lane_edges.len()];
    for (path_index, path) in maneuver_paths.iter().enumerate() {
        let path_edges = &maneuver_path_edges[path.edges.as_usize_range()];
        let [from, _to, ..] = path_edges else {
            unreachable!("validated ManeuverPath must contain at least entry and exit edges")
        };
        for pair in path_edges.windows(2) {
            let [transition_from, transition_to] = pair else {
                unreachable!("path transition windows always contain two edges")
            };
            edge_has_path_transition[transition_from.target.index()] = 1;
            let successor_range = lane_edges
                .get(transition_from.target)
                .successors
                .as_usize_range();
            if let Some(successor_offset) = lane_edge_references[successor_range.clone()]
                .iter()
                .position(|successor| successor.target == transition_to.target)
            {
                successor_has_path[successor_range.start + successor_offset] = 1;
            }
        }

        let Some(stop_key) = stop_line_by_edge[from.target.index()] else {
            continue;
        };
        if stop_has_entry_gate[stop_key.index()] == 0 || path_has_entry_gate[path_index] != 0 {
            continue;
        }
        let stop = stop_lines.get(stop_key);
        let mut diagnostic = Diagnostic::missing_maneuver_gate_coverage(
            &stop.stable_key,
            &lane_edges.get(from.target).stable_key,
            &path.stable_key,
            stop.source_span.clone(),
            path.source_span.clone(),
        );
        diagnostic.set_canonical_module_order(stop.module.raw());
        diagnostics.push(diagnostic);
    }
    for (stop_key, stop) in stop_lines.iter() {
        let successor_range = lane_edges.get(stop.lane_edge).successors.as_usize_range();
        if successor_range.is_empty() && edge_has_path_transition[stop.lane_edge.index()] == 0 {
            let mut diagnostic = Diagnostic::orphan_stop_line(
                &stop.stable_key,
                &lane_edges.get(stop.lane_edge).stable_key,
                stop.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(stop.module.raw());
            diagnostics.push(diagnostic);
        } else if stop_gate_counts[stop_key.index()] == 0 {
            let mut diagnostic = Diagnostic::unreferenced_stop_line(
                &stop.stable_key,
                &lane_edges.get(stop.lane_edge).stable_key,
                stop.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(stop.module.raw());
            diagnostics.push(diagnostic);
        } else if stop_has_entry_gate[stop_key.index()] != 0 {
            for successor_index in successor_range {
                if successor_has_path[successor_index] != 0 {
                    continue;
                }
                let successor = lane_edge_references[successor_index].target;
                let mut diagnostic = Diagnostic::missing_maneuver_path_coverage(
                    &stop.stable_key,
                    &lane_edges.get(stop.lane_edge).stable_key,
                    &lane_edges.get(successor).stable_key,
                    stop.source_span.clone(),
                    lane_edges.get(successor).source_span.clone(),
                );
                diagnostic.set_canonical_module_order(stop.module.raw());
                diagnostics.push(diagnostic);
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    drop(stop_line_by_edge);
    drop(path_has_entry_gate);
    drop(stop_has_entry_gate);
    drop(successor_has_path);
    drop(edge_has_path_transition);

    let mut path_gate_total = 0_usize;
    for (index, count) in path_gate_counts.iter().copied().enumerate() {
        maneuver_paths[index].maneuver_gates =
            TableRange::try_from_usize(path_gate_total, count)
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        path_gate_total = path_gate_total.saturating_add(count);
    }
    let mut maneuver_path_gates = Vec::with_capacity(path_gate_total);
    for gate_key in &resolved_gate_keys {
        maneuver_path_gates.push(HirManeuverPathGate {
            maneuver_gate: *gate_key,
        });
    }
    debug_assert_eq!(maneuver_path_gates.len(), path_gate_total);

    let mut stop_gate_order = resolved_gate_keys.clone();
    stop_gate_order.sort_unstable_by(|left, right| {
        let left = gates.get(*left);
        let right = gates.get(*right);
        (left.stop_line.raw(), left.stable_id).cmp(&(right.stop_line.raw(), right.stable_id))
    });
    let mut stop_gate_total = 0_usize;
    for (index, count) in stop_gate_counts.iter().copied().enumerate() {
        let key = HirStopLineKey::from_raw(u32::try_from(index).unwrap_or(u32::MAX));
        stop_lines.get_mut(key).maneuver_gates = TableRange::try_from_usize(stop_gate_total, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        stop_gate_total = stop_gate_total.saturating_add(count);
    }
    let stop_line_maneuver_gates = stop_gate_order
        .into_iter()
        .map(|maneuver_gate| HirStopLineManeuverGate { maneuver_gate })
        .collect::<Vec<_>>();

    let mut resolved_waiting_keys = Vec::with_capacity(waiting_zones.len());
    for location in &waiting_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::WaitingZone(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical WaitingZone source changed kind")
        };
        let path = resolve_reference(
            module_lookup,
            &path_symbols,
            &source.maneuver_path,
            EntityKind::WaitingZone,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let entry_gate = resolve_reference(
            module_lookup,
            &gate_symbols,
            &source.entry_gate,
            EntityKind::WaitingZone,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let release_gate = resolve_reference(
            module_lookup,
            &gate_symbols,
            &source.release_gate,
            EntityKind::WaitingZone,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let (Some(path_key), Some(entry_key), Some(release_key)) = (path, entry_gate, release_gate)
        else {
            continue;
        };
        let entry = gates.get(entry_key);
        let release = gates.get(release_key);
        let mut path_mismatch = false;
        for (role, gate) in [
            (WaitingZoneGateRole::Entry, entry),
            (WaitingZoneGateRole::Release, release),
        ] {
            if gate.maneuver_path != path_key {
                let mut diagnostic = Diagnostic::waiting_zone_gate_path_mismatch(
                    &source.header.stable_key,
                    role,
                    &gate.stable_key,
                    &maneuver_paths[path_key.index()].stable_key,
                    &maneuver_paths[gate.maneuver_path.index()].stable_key,
                    source.header.span.clone(),
                    gate.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
                path_mismatch = true;
            }
        }
        if path_mismatch {
            continue;
        }
        if entry.transition_index >= release.transition_index {
            let mut diagnostic = Diagnostic::invalid_waiting_zone_gate_order(
                &source.header.stable_key,
                entry.transition_index,
                release.transition_index,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
            continue;
        }
        let path_id = maneuver_paths[path_key.index()].stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::ManeuverPathStableId, path_id.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::WaitingZoneKey,
                source.header.stable_key.as_bytes(),
            ),
        ];
        let stable_id = WaitingZoneId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::WaitingZone,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let waiting = waiting_zones.get_mut(location.hir_key);
        waiting.stable_id = stable_id;
        waiting.maneuver_path = path_key;
        waiting.maneuver_path_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.maneuver_path.span,
        )?);
        waiting.entry_gate = entry_key;
        waiting.release_gate = release_key;
        resolved_waiting_keys.push(location.hir_key);
    }
    resolved_waiting_keys.sort_unstable_by(|left, right| {
        let left = waiting_zones.get(*left);
        let right = waiting_zones.get(*right);
        let left_entry = gates.get(left.entry_gate).transition_index;
        let right_entry = gates.get(right.entry_gate).transition_index;
        let left_release = gates.get(left.release_gate).transition_index;
        let right_release = gates.get(right.release_gate).transition_index;
        (
            left.maneuver_path.raw(),
            left_entry,
            left_release,
            left.stable_id,
        )
            .cmp(&(
                right.maneuver_path.raw(),
                right_entry,
                right_release,
                right.stable_id,
            ))
    });
    let mut active: Option<(HirWaitingZoneKey, u32)> = None;
    for waiting_key in &resolved_waiting_keys {
        let waiting = waiting_zones.get(*waiting_key);
        let entry = gates.get(waiting.entry_gate).transition_index;
        let release = gates.get(waiting.release_gate).transition_index;
        if let Some((active_key, active_release)) = active {
            let first = waiting_zones.get(active_key);
            if first.maneuver_path == waiting.maneuver_path && entry < active_release {
                let mut diagnostic = Diagnostic::overlapping_waiting_zones(
                    &maneuver_paths[waiting.maneuver_path.index()].stable_key,
                    &first.stable_key,
                    &waiting.stable_key,
                    waiting.source_span.clone(),
                    first.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(waiting.module.raw());
                diagnostics.push(diagnostic);
            }
            if first.maneuver_path != waiting.maneuver_path || release > active_release {
                active = Some((*waiting_key, release));
            }
        } else {
            active = Some((*waiting_key, release));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut path_waiting_counts = vec![0_usize; maneuver_paths.len()];
    for waiting_key in &resolved_waiting_keys {
        let path = waiting_zones.get(*waiting_key).maneuver_path;
        path_waiting_counts[path.index()] = path_waiting_counts[path.index()].saturating_add(1);
    }
    let mut waiting_total = 0_usize;
    for (index, count) in path_waiting_counts.iter().copied().enumerate() {
        maneuver_paths[index].waiting_zones = TableRange::try_from_usize(waiting_total, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        waiting_total = waiting_total.saturating_add(count);
    }
    let maneuver_path_waiting_zones = resolved_waiting_keys
        .iter()
        .copied()
        .map(|waiting_zone| HirManeuverPathWaitingZone { waiting_zone })
        .collect::<Vec<_>>();

    Ok(ControlHir {
        stop_lines: stop_lines.into_boxed_slice(),
        maneuver_gates: gates.into_boxed_slice(),
        waiting_zones: waiting_zones.into_boxed_slice(),
        maneuver_path_gates: maneuver_path_gates.into_boxed_slice(),
        maneuver_path_waiting_zones: maneuver_path_waiting_zones.into_boxed_slice(),
        stop_line_maneuver_gates: stop_line_maneuver_gates.into_boxed_slice(),
    })
}
