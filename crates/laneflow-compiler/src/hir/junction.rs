//! 路口领域 HIR：路口、通行流向、机动路径与路口内部边的记录与构建。

use core::hash::{Hash, Hasher};
use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, FieldTag, JunctionId, ManeuverPathId, MovementId, StableId128,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{TypedAstDeclaration, TypedAstEntityAddress};
use crate::diagnostic::{DiagnosticCollector, JunctionEdgeSetViolation};
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    CanonicalDeclarationSource, HirAuthoringLaneEdge, HirJunctionKey, HirJunctionTag, HirLaneEdge,
    HirLaneEdgeKey, HirLaneEdgeReference, HirLaneEdgeTag, HirManeuverPathGate, HirManeuverPathKey,
    HirManeuverPathTag, HirManeuverPathWaitingZone, HirModuleKey, HirMovementKey, HirMovementTag,
    JunctionCounts, SymbolTable, arena_overflow, count_to_usize, declaration_header,
    derive_identity, resolve_reference,
};

/// 已解析出非空通行流向成员区间的路口。
#[derive(Debug, PartialEq)]
pub(crate) struct HirJunction {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: JunctionId,
    pub(crate) movements: TableRange<HirJunctionMovement>,
    pub(crate) source_span: SourceLocation,
}

/// 已闭合到唯一路口父项并保留 Identity v1 有向引道键的通行流向。
#[derive(Debug, PartialEq)]
pub(crate) struct HirMovement {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: MovementId,
    pub(crate) junction: HirJunctionKey,
    pub(crate) junction_source_location: Option<ResolvedSourceLocation>,
    pub(crate) directed_entry_approach_key: Arc<str>,
    pub(crate) directed_exit_approach_key: Arc<str>,
    pub(crate) turn_direction: Option<laneflow_static_contract::ManeuverDirection>,
    pub(crate) direction_source: Option<SourceLocation>,
    pub(crate) maneuver_paths: TableRange<HirMovementManeuverPath>,
    pub(crate) source_span: SourceLocation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirJunctionMovement {
    pub(crate) movement: HirMovementKey,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirMovementManeuverPath {
    pub(crate) maneuver_path: HirManeuverPathKey,
}

/// 一条机动路径完整遍历序列中的已解析车道图边。
#[derive(Debug, PartialEq)]
pub(crate) struct HirManeuverPathEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceLocation,
}

/// 已解析父项、入口/内部/出口边和全局唯一遍历序列的机动路径。
#[derive(Debug, PartialEq)]
pub(crate) struct HirManeuverPath {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    pub(crate) stable_id: ManeuverPathId,
    pub(crate) movement: HirMovementKey,
    pub(crate) movement_source_location: Option<ResolvedSourceLocation>,
    /// 完整序列 `entry + internal + exit`；首尾是边界边，中间区间是内部边。
    pub(crate) edges: TableRange<HirManeuverPathEdge>,
    /// 按 `transition_index` 严格递增的机动门成员区间。
    pub(crate) maneuver_gates: TableRange<HirManeuverPathGate>,
    /// 按入口转换、释放转换和稳定 ID 排序的等待区成员区间。
    pub(crate) waiting_zones: TableRange<HirManeuverPathWaitingZone>,
    pub(crate) source_span: SourceLocation,
}

/// 从全部路径派生的路口内部边规范代表声明。
#[derive(Debug, PartialEq)]
pub(crate) struct HirJunctionInternalEdge {
    pub(crate) edge: HirLaneEdgeKey,
    pub(crate) junction: HirJunctionKey,
    /// 多条路径共享同一内部边时按 StableId 选择的代表路径，供诊断回链与路线闭包使用。
    pub(crate) source_path: HirManeuverPathKey,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct JunctionHir {
    pub(crate) junctions: Box<[HirJunction]>,
    pub(crate) movements: Box<[HirMovement]>,
    pub(crate) junction_movements: Box<[HirJunctionMovement]>,
    pub(crate) maneuver_paths: Box<[HirManeuverPath]>,
    pub(crate) movement_maneuver_paths: Box<[HirMovementManeuverPath]>,
    pub(crate) maneuver_path_edges: Box<[HirManeuverPathEdge]>,
    pub(crate) junction_internal_edges: Box<[HirJunctionInternalEdge]>,
}

#[derive(Clone)]
pub(crate) struct HirDeclaredJunctionEdge {
    junction: HirJunctionKey,
    edge: HirLaneEdgeKey,
    source_span: SourceLocation,
}

fn find_declared_junction_edge(
    values: &[HirDeclaredJunctionEdge],
    junction: HirJunctionKey,
    edge: HirLaneEdgeKey,
) -> Option<&HirDeclaredJunctionEdge> {
    values
        .binary_search_by_key(&(junction, edge), |value| (value.junction, value.edge))
        .ok()
        .map(|index| &values[index])
}

/// 只在完成路径表后借用的序列查找键；来源位置不参与遍历签名。
pub(crate) struct ManeuverPathSequence<'a>(&'a [HirManeuverPathEdge]);

impl PartialEq for ManeuverPathSequence<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .map(|edge| edge.target)
            .eq(other.0.iter().map(|edge| edge.target))
    }
}

impl Eq for ManeuverPathSequence<'_> {}

impl Hash for ManeuverPathSequence<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for edge in self.0 {
            edge.target.hash(state);
        }
    }
}

#[allow(clippy::too_many_lines)]
// Every parameter is one explicit HIR stage dependency; an aggregate context would broaden
// access and make the stage boundary less reviewable.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_junction_hir(
    unit: &CompilationUnit,
    counts: &JunctionCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    authoring_lane_edges: &[HirAuthoringLaneEdge],
    identities: &mut IdentityRegistry,
) -> Result<JunctionHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(JunctionHir::default());
    }

    let junction_capacity = count_to_usize(counts.junctions, &unit.limits)?;
    let movement_capacity = count_to_usize(counts.movements, &unit.limits)?;
    let path_capacity = count_to_usize(counts.maneuver_paths, &unit.limits)?;
    let mut junctions = TypedArena::<HirJunctionTag, HirJunction>::with_capacity(junction_capacity);
    let mut movements = TypedArena::<HirMovementTag, HirMovement>::with_capacity(movement_capacity);
    let mut paths = TypedArena::<HirManeuverPathTag, HirManeuverPath>::with_capacity(path_capacity);
    let mut junction_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::Junction(_)))
            .count()
    }));
    let mut movement_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::Movement(_)))
            .count()
    }));
    let mut movement_sources = Vec::with_capacity(movement_capacity);
    let mut path_sources = Vec::with_capacity(path_capacity);

    // 三种声明都先按模块和稳定键分配完整符号。只有 Junction 不依赖父项，可以立即
    // 派生身份；Movement 与 ManeuverPath 先写入占位值，随后按父项顺序闭合。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declaration_indices: Vec<_> = (0..source_module.declarations.len()).collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), &left.source_address)
                .cmp(&(right.entity_kind.code(), &right.source_address))
        });
        for declaration_index in declaration_indices {
            match &source_module.declarations[declaration_index] {
                TypedAstDeclaration::Junction(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::JunctionKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = JunctionId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::Junction,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = junctions
                        .push(HirJunction {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            movements: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    junction_symbols.insert(module_key, source.header.source_address.clone(), key);
                }
                TypedAstDeclaration::Movement(source) => {
                    let key = movements
                        .push(HirMovement {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: MovementId::from_untyped(StableId128::ZERO),
                            junction: HirJunctionKey::from_raw(0),
                            junction_source_location: None,
                            directed_entry_approach_key: Arc::clone(
                                &source.directed_entry_approach_key,
                            ),
                            directed_exit_approach_key: Arc::clone(
                                &source.directed_exit_approach_key,
                            ),
                            turn_direction: source.turn_direction,
                            direction_source: source.direction_source.clone(),
                            maneuver_paths: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    movement_symbols.insert(module_key, source.header.source_address.clone(), key);
                    movement_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: u32::try_from(declaration_index).map_err(|_| {
                            arena_overflow(
                                ArenaKeyOverflow,
                                &unit.limits,
                                Some(source.header.span.clone()),
                            )
                        })?,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::ManeuverPath(source) => {
                    let key = paths
                        .push(HirManeuverPath {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            source_address: source.header.source_address.clone(),
                            stable_id: ManeuverPathId::from_untyped(StableId128::ZERO),
                            movement: HirMovementKey::from_raw(0),
                            movement_source_location: None,
                            edges: TableRange::empty(),
                            maneuver_gates: TableRange::empty(),
                            waiting_zones: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    path_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: u32::try_from(declaration_index).map_err(|_| {
                            arena_overflow(
                                ArenaKeyOverflow,
                                &unit.limits,
                                Some(source.header.span.clone()),
                            )
                        })?,
                        hir_key: key,
                    });
                }
                _ => {}
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut section_derived_edges = vec![0_u8; lane_edges.len()];
    for edge in authoring_lane_edges {
        section_derived_edges[edge.target.index()] = 1;
    }
    let mut declared_approaches = Vec::with_capacity(count_to_usize(
        counts.declared_approach_edges,
        &unit.limits,
    )?);
    let mut declared_internal_edges = Vec::with_capacity(count_to_usize(
        counts.declared_internal_edges,
        &unit.limits,
    )?);
    // RoadEditingSource 的 approach/internal vectors 是完整显式集合。Synthetic Junction
    // 的两个集合为空，继续沿用仅由路径派生角色的历史输入语义。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("compile limits bound module ordinals"),
        );
        for declaration in &source_module.declarations {
            let TypedAstDeclaration::Junction(source) = declaration else {
                continue;
            };
            let junction = junction_symbols
                .get(module_key, &source.header.source_address)
                .expect("every canonical Junction has a symbol");
            for approach in &source.approach_edges {
                let Some(edge) = resolve_reference(
                    module_lookup,
                    lane_edge_symbols,
                    approach,
                    EntityKind::Junction,
                    &source.header,
                    u32::try_from(module_index).unwrap_or(u32::MAX),
                    &mut diagnostics,
                ) else {
                    continue;
                };
                if section_derived_edges[edge.index()] == 0 {
                    let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
                        &source.header.stable_key,
                        &lane_edges.get(edge).stable_key,
                        None,
                        JunctionEdgeSetViolation::ApproachNotSectionDerived,
                        approach.span.clone(),
                        Some(source.header.span.clone()),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                }
                declared_approaches.push(HirDeclaredJunctionEdge {
                    junction,
                    edge,
                    source_span: approach.span.clone(),
                });
            }
            for internal in &source.internal_edges {
                let Some(edge) = resolve_reference(
                    module_lookup,
                    lane_edge_symbols,
                    internal,
                    EntityKind::Junction,
                    &source.header,
                    u32::try_from(module_index).unwrap_or(u32::MAX),
                    &mut diagnostics,
                ) else {
                    continue;
                };
                if section_derived_edges[edge.index()] != 0 {
                    let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
                        &source.header.stable_key,
                        &lane_edges.get(edge).stable_key,
                        None,
                        JunctionEdgeSetViolation::InternalIsSectionDerived,
                        internal.span.clone(),
                        Some(source.header.span.clone()),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    diagnostics.push(diagnostic);
                }
                declared_internal_edges.push(HirDeclaredJunctionEdge {
                    junction,
                    edge,
                    source_span: internal.span.clone(),
                });
            }
        }
    }
    declared_approaches.sort_unstable_by_key(|value| (value.junction, value.edge));
    declared_internal_edges.sort_unstable_by_key(|value| (value.junction, value.edge));
    let mut junction_member_counts = vec![0_usize; junctions.len()];
    for location in &movement_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let source =
            movement_declaration(&source_module.declarations[location.declaration_index as usize])
                .expect("canonical Movement source must name a Movement");
        let Some(junction) = resolve_reference(
            module_lookup,
            &junction_symbols,
            &source.junction,
            EntityKind::Movement,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) else {
            continue;
        };
        let junction_id = junctions.get(junction).stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::MovementKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::DirectedEntryApproachKey,
                source.directed_entry_approach_key.as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::DirectedExitApproachKey,
                source.directed_exit_approach_key.as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::JunctionStableId, junction_id.as_bytes()),
        ];
        let stable_id = MovementId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::Movement,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let movement = movements.get_mut(location.hir_key);
        movement.stable_id = stable_id;
        movement.junction = junction;
        movement.junction_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.junction.span,
        )?);
        junction_member_counts[junction.index()] =
            junction_member_counts[junction.index()].saturating_add(1);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut path_edges =
        Vec::with_capacity(count_to_usize(counts.maneuver_path_edges, &unit.limits)?);
    let mut movement_member_counts = vec![0_usize; movements.len()];
    for location in &path_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let source = maneuver_path_declaration(
            &source_module.declarations[location.declaration_index as usize],
        )
        .expect("canonical ManeuverPath source must name a ManeuverPath");
        let movement = resolve_reference(
            module_lookup,
            &movement_symbols,
            &source.movement,
            EntityKind::ManeuverPath,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let start = path_edges.len();
        let mut entry = None;
        let mut exit = None;
        for (index, reference) in core::iter::once(&source.entry_edge)
            .chain(source.internal_edges.iter())
            .chain(core::iter::once(&source.exit_edge))
            .enumerate()
        {
            let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::ManeuverPath,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            ) else {
                continue;
            };
            if index == 0 {
                entry = Some(target);
            }
            if index == source.internal_edges.len().saturating_add(1) {
                exit = Some(target);
            }
            path_edges.push(HirManeuverPathEdge {
                target,
                source_span: reference.span.clone(),
            });
        }
        let (Some(movement), Some(entry), Some(exit)) = (movement, entry, exit) else {
            continue;
        };
        let junction = movements.get(movement).junction;
        let has_explicit_edge_contract = declared_approaches
            .binary_search_by_key(&junction, |value| value.junction)
            .is_ok()
            || declared_internal_edges
                .binary_search_by_key(&junction, |value| value.junction)
                .is_ok();
        if has_explicit_edge_contract {
            let edges = &path_edges[start..];
            for (local_index, edge) in edges.iter().enumerate() {
                let is_boundary = local_index == 0 || local_index + 1 == edges.len();
                let declared = if is_boundary {
                    find_declared_junction_edge(&declared_approaches, junction, edge.target)
                } else {
                    find_declared_junction_edge(&declared_internal_edges, junction, edge.target)
                };
                if declared.is_none() {
                    let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
                        &junctions.get(junction).stable_key,
                        &lane_edges.get(edge.target).stable_key,
                        Some(&source.header.stable_key),
                        if is_boundary {
                            JunctionEdgeSetViolation::BoundaryNotDeclaredApproach
                        } else {
                            JunctionEdgeSetViolation::InternalNotDeclared
                        },
                        edge.source_span.clone(),
                        Some(junctions.get(junction).source_span.clone()),
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                }
            }
        }
        let movement_id = movements.get(movement).stable_id.into_untyped();
        let entry_id = lane_edges.get(entry).stable_id.into_untyped();
        let exit_id = lane_edges.get(exit).stable_id.into_untyped();
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::PathKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(FieldTag::MovementStableId, movement_id.as_bytes()),
            IdentityFieldInput::new(FieldTag::EntryEdgeStableId, entry_id.as_bytes()),
            IdentityFieldInput::new(FieldTag::ExitEdgeStableId, exit_id.as_bytes()),
        ];
        let stable_id = ManeuverPathId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::ManeuverPath,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let path = paths.get_mut(location.hir_key);
        path.stable_id = stable_id;
        path.movement = movement;
        path.movement_source_location = Some(unit.resolve_source_location_for_module(
            location.source_module_index,
            &source.movement.span,
        )?);
        path.edges = TableRange::try_from_usize(start, path_edges.len().saturating_sub(start))
            .map_err(|overflow| {
                arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
            })?;
        movement_member_counts[movement.index()] =
            movement_member_counts[movement.index()].saturating_add(1);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 完整路径序列的全局唯一性先于内部边角色派生，以保持 zero-internal 与普通路径的
    // 重复错误一致。HashMap 只查找已冻结切片，完整目标键比较封堵哈希碰撞。
    let mut sequence_index: HashMap<ManeuverPathSequence<'_>, HirManeuverPathKey> =
        HashMap::with_capacity(paths.len());
    for (path_key, path) in paths.iter() {
        let sequence = ManeuverPathSequence(&path_edges[path.edges.as_usize_range()]);
        if let Some(first_path_key) = sequence_index.get(&sequence).copied() {
            let first = paths.get(first_path_key);
            let first_junction = movements.get(first.movement).junction;
            let duplicate_junction = movements.get(path.movement).junction;
            let mut diagnostic = Diagnostic::duplicate_maneuver_path_sequence(
                &first.stable_key,
                &path.stable_key,
                &junctions.get(first_junction).stable_key,
                &junctions.get(duplicate_junction).stable_key,
                path.source_span.clone(),
                first.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(path.module.raw());
            diagnostics.push(diagnostic);
        } else {
            sequence_index.insert(sequence, path_key);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }
    drop(sequence_index);

    let mut internal_claims: Vec<Option<HirJunctionInternalEdge>> =
        (0..lane_edges.len()).map(|_| None).collect();
    let mut boundary_claims: Vec<Option<(HirManeuverPathKey, SourceLocation)>> =
        (0..lane_edges.len()).map(|_| None).collect();
    for (path_key, path) in paths.iter() {
        let edge_range = path.edges.as_usize_range();
        let edges = &path_edges[edge_range];
        let junction = movements.get(path.movement).junction;
        for (local_index, edge) in edges.iter().enumerate() {
            let is_boundary = local_index == 0 || local_index + 1 == edges.len();
            if is_boundary {
                if let Some(internal) = &internal_claims[edge.target.index()] {
                    let internal_path = paths.get(internal.source_path);
                    let mut diagnostic = Diagnostic::internal_boundary_role_conflict(
                        &lane_edges.get(edge.target).stable_key,
                        &internal_path.stable_key,
                        &path.stable_key,
                        edge.source_span.clone(),
                        internal.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(path.module.raw());
                    diagnostics.push(diagnostic);
                } else if boundary_claims[edge.target.index()].is_none() {
                    boundary_claims[edge.target.index()] =
                        Some((path_key, edge.source_span.clone()));
                }
                continue;
            }
            if let Some((boundary_path_key, boundary_span)) = &boundary_claims[edge.target.index()]
            {
                let mut diagnostic = Diagnostic::internal_boundary_role_conflict(
                    &lane_edges.get(edge.target).stable_key,
                    &path.stable_key,
                    &paths.get(*boundary_path_key).stable_key,
                    edge.source_span.clone(),
                    boundary_span.clone(),
                );
                diagnostic.set_canonical_module_order(path.module.raw());
                diagnostics.push(diagnostic);
                continue;
            }
            if let Some(first) = &internal_claims[edge.target.index()] {
                if first.junction != junction {
                    let mut diagnostic = Diagnostic::internal_edge_junction_conflict(
                        &lane_edges.get(edge.target).stable_key,
                        &junctions.get(first.junction).stable_key,
                        &junctions.get(junction).stable_key,
                        &paths.get(first.source_path).stable_key,
                        &path.stable_key,
                        edge.source_span.clone(),
                        first.source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(path.module.raw());
                    diagnostics.push(diagnostic);
                } else if path.stable_id < paths.get(first.source_path).stable_id {
                    // 同一路口多条路径可共享内部边。来源映射选择 StableId 较小的路径作为
                    // 规范主要来源，避免声明排列改变同一派生关系的回链位置。
                    internal_claims[edge.target.index()] = Some(HirJunctionInternalEdge {
                        edge: edge.target,
                        junction,
                        source_path: path_key,
                        source_span: edge.source_span.clone(),
                    });
                }
            } else {
                internal_claims[edge.target.index()] = Some(HirJunctionInternalEdge {
                    edge: edge.target,
                    junction,
                    source_path: path_key,
                    source_span: edge.source_span.clone(),
                });
            }
        }
    }

    // RoadEditingSource 的显式集合是路口角色的完整闭包：任何 approach 都不能在任一
    // 路口被路径声明为 internal，且每条显式 internal edge 都必须由同一路口至少一条
    // 路径实际使用。Synthetic Junction 没有显式集合，因此自然不会进入这两轮检查。
    for declared in &declared_approaches {
        let Some(internal) = &internal_claims[declared.edge.index()] else {
            continue;
        };
        let internal_path = paths.get(internal.source_path);
        let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
            &junctions.get(declared.junction).stable_key,
            &lane_edges.get(declared.edge).stable_key,
            Some(&internal_path.stable_key),
            JunctionEdgeSetViolation::ApproachClaimedInternal,
            declared.source_span.clone(),
            Some(internal.source_span.clone()),
        );
        diagnostic.set_canonical_module_order(junctions.get(declared.junction).module.raw());
        diagnostics.push(diagnostic);
    }
    for declared in &declared_internal_edges {
        let claim = internal_claims[declared.edge.index()].as_ref();
        if claim.is_some_and(|claim| claim.junction == declared.junction) {
            continue;
        }
        let related_span = claim.map(|claim| claim.source_span.clone());
        let path_key = claim.map(|claim| paths.get(claim.source_path).stable_key.as_ref());
        let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
            &junctions.get(declared.junction).stable_key,
            &lane_edges.get(declared.edge).stable_key,
            path_key,
            JunctionEdgeSetViolation::DeclaredInternalUnused,
            declared.source_span.clone(),
            related_span,
        );
        diagnostic.set_canonical_module_order(junctions.get(declared.junction).module.raw());
        diagnostics.push(diagnostic);
    }
    for declared in &declared_internal_edges {
        let edge = lane_edges.get(declared.edge);
        let Some(successor) = lane_edge_references[edge.successors.as_usize_range()].first() else {
            continue;
        };
        let path_key = internal_claims[declared.edge.index()]
            .as_ref()
            .map(|claim| paths.get(claim.source_path).stable_key.as_ref());
        let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
            &junctions.get(declared.junction).stable_key,
            &edge.stable_key,
            path_key,
            JunctionEdgeSetViolation::InternalHasSuccessors,
            successor.source_span.clone(),
            Some(declared.source_span.clone()),
        );
        diagnostic.set_canonical_module_order(edge.module.raw());
        diagnostics.push(diagnostic);
    }
    for (edge_key, edge) in lane_edges.iter() {
        let owner_is_explicit_internal =
            internal_claims[edge_key.index()]
                .as_ref()
                .is_some_and(|claim| {
                    find_declared_junction_edge(&declared_internal_edges, claim.junction, edge_key)
                        .is_some()
                });
        if owner_is_explicit_internal {
            // The owner-side check above already rejects every successor on an explicit internal
            // edge. Avoid producing a second diagnostic when its target is also internal.
            continue;
        }
        for successor in &lane_edge_references[edge.successors.as_usize_range()] {
            let Some(claim) = internal_claims[successor.target.index()].as_ref() else {
                continue;
            };
            let Some(declared) = find_declared_junction_edge(
                &declared_internal_edges,
                claim.junction,
                successor.target,
            ) else {
                continue;
            };
            let mut diagnostic = Diagnostic::junction_edge_set_mismatch(
                &junctions.get(claim.junction).stable_key,
                &lane_edges.get(successor.target).stable_key,
                Some(&paths.get(claim.source_path).stable_key),
                JunctionEdgeSetViolation::InternalReferencedBySuccessor,
                successor.source_span.clone(),
                Some(declared.source_span.clone()),
            );
            diagnostic.set_canonical_module_order(edge.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // Junction-internal edges intentionally carry no `successors`: the ManeuverPath sequence is
    // the sole topology authority for every transition that touches one. A path with no internal
    // edge is an ordinary section-to-section transition and must still be backed by a declared
    // successor, preserving the non-junction lane graph contract.
    for (_, path) in paths.iter() {
        let edges = &path_edges[path.edges.as_usize_range()];
        let junction = movements.get(path.movement).junction;
        for pair in edges.windows(2) {
            let [predecessor, successor] = pair else {
                unreachable!("windows(2) always yields two elements")
            };
            if find_declared_junction_edge(&declared_internal_edges, junction, predecessor.target)
                .is_some()
                || find_declared_junction_edge(&declared_internal_edges, junction, successor.target)
                    .is_some()
            {
                continue;
            }
            let predecessor_record = lane_edges.get(predecessor.target);
            let connected = lane_edge_references[predecessor_record.successors.as_usize_range()]
                .iter()
                .any(|candidate| candidate.target == successor.target);
            if connected {
                continue;
            }
            let mut diagnostic = Diagnostic::disconnected_maneuver_path(
                &path.stable_key,
                &predecessor_record.stable_key,
                &lane_edges.get(successor.target).stable_key,
                successor.source_span.clone(),
                predecessor.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(path.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    for (junction_key, junction) in junctions.iter() {
        if junction_member_counts[junction_key.index()] == 0 {
            let mut diagnostic =
                Diagnostic::empty_junction(&junction.stable_key, junction.source_span.clone());
            diagnostic.set_canonical_module_order(junction.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    for (movement_key, movement) in movements.iter() {
        if movement_member_counts[movement_key.index()] == 0 {
            let mut diagnostic =
                Diagnostic::empty_movement(&movement.stable_key, movement.source_span.clone());
            diagnostic.set_canonical_module_order(movement.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut next_junction_member = Vec::with_capacity(junctions.len());
    let mut junction_member_total = 0_usize;
    for (index, count) in junction_member_counts.iter().copied().enumerate() {
        next_junction_member.push(junction_member_total);
        let key = HirJunctionKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        junctions.get_mut(key).movements = TableRange::try_from_usize(junction_member_total, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        junction_member_total = junction_member_total.saturating_add(count);
    }
    let first_movement = movements
        .iter()
        .next()
        .map(|(key, _)| key)
        .unwrap_or(HirMovementKey::from_raw(0));
    let mut junction_movements = vec![
        HirJunctionMovement {
            movement: first_movement,
        };
        junction_member_total
    ];
    for (movement_key, movement) in movements.iter() {
        let destination = &mut next_junction_member[movement.junction.index()];
        junction_movements[*destination] = HirJunctionMovement {
            movement: movement_key,
        };
        *destination += 1;
    }

    let mut next_movement_member = Vec::with_capacity(movements.len());
    let mut movement_member_total = 0_usize;
    for (index, count) in movement_member_counts.iter().copied().enumerate() {
        next_movement_member.push(movement_member_total);
        let key = HirMovementKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        movements.get_mut(key).maneuver_paths =
            TableRange::try_from_usize(movement_member_total, count)
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        movement_member_total = movement_member_total.saturating_add(count);
    }
    let first_path = paths
        .iter()
        .next()
        .map(|(key, _)| key)
        .unwrap_or(HirManeuverPathKey::from_raw(0));
    let mut movement_maneuver_paths = vec![
        HirMovementManeuverPath {
            maneuver_path: first_path,
        };
        movement_member_total
    ];
    for (path_key, path) in paths.iter() {
        let destination = &mut next_movement_member[path.movement.index()];
        movement_maneuver_paths[*destination] = HirMovementManeuverPath {
            maneuver_path: path_key,
        };
        *destination += 1;
    }

    let junction_internal_edges = internal_claims
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(JunctionHir {
        junctions: junctions.into_boxed_slice(),
        movements: movements.into_boxed_slice(),
        junction_movements: junction_movements.into_boxed_slice(),
        maneuver_paths: paths.into_boxed_slice(),
        movement_maneuver_paths: movement_maneuver_paths.into_boxed_slice(),
        maneuver_path_edges: path_edges.into_boxed_slice(),
        junction_internal_edges,
    })
}

fn movement_declaration(
    declaration: &TypedAstDeclaration,
) -> Option<&crate::declaration::MovementDeclaration> {
    match declaration {
        TypedAstDeclaration::Movement(declaration) => Some(declaration),
        _ => None,
    }
}

fn maneuver_path_declaration(
    declaration: &TypedAstDeclaration,
) -> Option<&crate::declaration::ManeuverPathDeclaration> {
    match declaration {
        TypedAstDeclaration::ManeuverPath(declaration) => Some(declaration),
        _ => None,
    }
}
