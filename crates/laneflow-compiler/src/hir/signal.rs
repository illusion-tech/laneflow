//! 信号（signal）领域 HIR：固定时制信号控制器、信号组与相位的记录与构建。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    EntityKind, FieldTag, SignalAspect, SignalControllerId, SignalGroupId, SignalPhaseId,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{MAX_PORTABLE_SIGNAL_TIME_MS, OwnedSignalControl, TypedAstDeclaration};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::module::ResolvedSourceLocation;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::{
    CanonicalDeclarationSource, HirManeuverGate, HirManeuverGateKey, HirModuleKey,
    HirSignalControllerKey, HirSignalControllerTag, HirSignalGroupKey, HirSignalGroupTag,
    HirSignalPhaseTag, SignalCounts, SymbolTable, arena_overflow, count_to_usize,
    declaration_header, derive_identity, resolve_reference,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HirSignalControl {
    Group {
        signal_group: HirSignalGroupKey,
        source_location: ResolvedSourceLocation,
    },
    None,
}

/// 由一个固定时制控制器唯一拥有、并至少控制一个机动门的信号组。
#[derive(Debug, PartialEq)]
pub(crate) struct HirSignalGroup {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalGroupId,
    pub(crate) controller: HirSignalControllerKey,
    pub(crate) maneuver_gates: TableRange<HirSignalGroupManeuverGate>,
    pub(crate) source_span: SourceLocation,
}

/// 一个信号组控制的机动门反向关系项。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HirSignalGroupManeuverGate {
    pub(crate) maneuver_gate: HirManeuverGateKey,
}

/// 控制器有序信号组列表中的一项。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HirSignalControllerGroup {
    pub(crate) signal_group: HirSignalGroupKey,
    pub(crate) source_location: ResolvedSourceLocation,
}

/// 固定时制控制器的不可变循环程序。
#[derive(Debug, PartialEq)]
pub(crate) struct HirSignalController {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalControllerId,
    pub(crate) offset_ms: u64,
    pub(crate) cycle_duration_ms: u64,
    pub(crate) signal_groups: TableRange<HirSignalControllerGroup>,
    pub(crate) phases: TableRange<HirSignalPhase>,
    pub(crate) source_span: SourceLocation,
}

/// 控制器所有者局部（owner-local）的一个有序固定时制相位。
#[derive(Debug, PartialEq)]
pub(crate) struct HirSignalPhase {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: SignalPhaseId,
    pub(crate) controller: HirSignalControllerKey,
    pub(crate) duration_ms: u64,
    /// 状态按所属控制器的 `signal_groups` 顺序规范化，而非按输入顺序保存。
    pub(crate) states: TableRange<HirSignalPhaseState>,
    pub(crate) controller_relation_source_location: ResolvedSourceLocation,
    pub(crate) source_span: SourceLocation,
}

/// 一个相位对其控制器信号组的完整灯色赋值。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HirSignalPhaseState {
    pub(crate) signal_group: HirSignalGroupKey,
    pub(crate) aspect: SignalAspect,
    pub(crate) source_location: ResolvedSourceLocation,
}

#[derive(Default)]
pub(crate) struct SignalHir {
    pub(crate) signal_groups: Box<[HirSignalGroup]>,
    pub(crate) signal_controllers: Box<[HirSignalController]>,
    pub(crate) signal_controller_groups: Box<[HirSignalControllerGroup]>,
    pub(crate) signal_phases: Box<[HirSignalPhase]>,
    pub(crate) signal_phase_states: Box<[HirSignalPhaseState]>,
    pub(crate) signal_group_maneuver_gates: Box<[HirSignalGroupManeuverGate]>,
}

#[allow(clippy::too_many_lines)]
pub(crate) fn build_signal_hir(
    unit: &CompilationUnit,
    counts: &SignalCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    maneuver_gates: &mut [HirManeuverGate],
    identities: &mut IdentityRegistry,
) -> Result<SignalHir, DiagnosticBundle> {
    if counts.entity_count() == 0 && counts.controlled_gates == 0 {
        return Ok(SignalHir::default());
    }

    let mut groups = TypedArena::<HirSignalGroupTag, HirSignalGroup>::with_capacity(
        count_to_usize(counts.groups, &unit.limits)?,
    );
    let mut controllers = TypedArena::<HirSignalControllerTag, HirSignalController>::with_capacity(
        count_to_usize(counts.controllers, &unit.limits)?,
    );
    let mut phases = TypedArena::<HirSignalPhaseTag, HirSignalPhase>::with_capacity(
        count_to_usize(counts.phases, &unit.limits)?,
    );
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::SignalGroup(_)))
            .count()
    }));
    let mut gate_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, TypedAstDeclaration::ManeuverGate(_)))
            .count()
    }));
    for (index, gate) in maneuver_gates.iter().enumerate() {
        let key = HirManeuverGateKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        gate_symbols.insert(gate.module, gate.source_address.clone(), key);
    }

    let mut group_sources = Vec::with_capacity(count_to_usize(counts.groups, &unit.limits)?);
    let mut controller_sources =
        Vec::with_capacity(count_to_usize(counts.controllers, &unit.limits)?);

    // 信号组和控制器先按规范模块顺序、模块内稳定键登记，随后才解析所有权和门绑定。
    // 因此控制器、相位或门都可以前向引用同一编译单元内的组。
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
                    TypedAstDeclaration::SignalGroup(_) | TypedAstDeclaration::SignalController(_)
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
                TypedAstDeclaration::SignalGroup(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::SignalGroupKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = SignalGroupId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::SignalGroup,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = groups
                        .push(HirSignalGroup {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            controller: HirSignalControllerKey::from_raw(0),
                            maneuver_gates: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    group_symbols.insert(module_key, source.header.source_address.clone(), key);
                    group_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                TypedAstDeclaration::SignalController(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::SignalControllerKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = SignalControllerId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::SignalController,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = controllers
                        .push(HirSignalController {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            offset_ms: source.offset_ms,
                            cycle_duration_ms: 0,
                            signal_groups: TableRange::empty(),
                            phases: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    controller_sources.push(CanonicalDeclarationSource {
                        source_module_index: module_order,
                        declaration_index: source_index,
                        hir_key: key,
                    });
                }
                _ => unreachable!("signal source filter admitted unrelated declaration"),
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut owners: Vec<Option<(HirSignalControllerKey, SourceLocation)>> =
        vec![None; groups.len()];
    let mut controller_group_rows =
        Vec::with_capacity(count_to_usize(counts.controller_groups, &unit.limits)?);
    let mut phase_states = Vec::with_capacity(count_to_usize(counts.phase_states, &unit.limits)?);

    for location in &controller_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::SignalController(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical SignalController source changed kind")
        };
        let module_order = location.source_module_index;
        let controller_key = location.hir_key;

        if source.signal_groups.is_empty() {
            let mut diagnostic = Diagnostic::empty_signal_controller_groups(
                &source.header.stable_key,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
        }
        if source.phases.is_empty() {
            let mut diagnostic = Diagnostic::empty_signal_controller_phases(
                &source.header.stable_key,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
        }

        let mut resolved_groups = Vec::with_capacity(source.signal_groups.len());
        let mut first_group_spans =
            HashMap::<HirSignalGroupKey, SourceLocation>::with_capacity(source.signal_groups.len());
        for reference in &source.signal_groups {
            let Some(group_key) = resolve_reference(
                module_lookup,
                &group_symbols,
                reference,
                EntityKind::SignalController,
                &source.header,
                module_order,
                &mut diagnostics,
            ) else {
                continue;
            };
            if let Some(first_span) = first_group_spans.get(&group_key) {
                let mut diagnostic = Diagnostic::duplicate_signal_controller_group(
                    &source.header.stable_key,
                    &groups.get(group_key).stable_key,
                    reference.span.clone(),
                    first_span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
                continue;
            }
            first_group_spans.insert(group_key, reference.span.clone());
            if let Some((first_controller, first_span)) = &owners[group_key.index()] {
                let mut diagnostic = Diagnostic::signal_group_multiple_controllers(
                    &groups.get(group_key).stable_key,
                    &controllers.get(*first_controller).stable_key,
                    &source.header.stable_key,
                    reference.span.clone(),
                    first_span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            } else {
                owners[group_key.index()] = Some((controller_key, reference.span.clone()));
                groups.get_mut(group_key).controller = controller_key;
            }
            resolved_groups.push(group_key);
        }
        // 控制器的组声明是集合语义；这里按 StableId 建立 HIR 阶段局部确定顺序，
        // 只用于消除来源排列，不能把它当作最终 LIR 的完整身份顺序。
        resolved_groups.sort_unstable_by_key(|key| groups.get(*key).stable_id);
        let group_start = controller_group_rows.len();
        for signal_group in resolved_groups.iter().copied() {
            let source_span = first_group_spans
                .get(&signal_group)
                .expect("resolved controller group retains its first reference span");
            controller_group_rows.push(HirSignalControllerGroup {
                signal_group,
                source_location: unit
                    .resolve_source_location_for_module(module_order, source_span)?,
            });
        }
        controllers.get_mut(controller_key).signal_groups = TableRange::try_from_usize(
            group_start,
            controller_group_rows.len().saturating_sub(group_start),
        )
        .map_err(|overflow| {
            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
        })?;

        let group_positions: HashMap<_, _> = resolved_groups
            .iter()
            .copied()
            .enumerate()
            .map(|(position, key)| (key, position))
            .collect();
        let phase_start = phases.len();
        let mut phase_keys =
            HashMap::<Arc<str>, SourceLocation>::with_capacity(source.phases.len());
        let mut cycle_duration_ms = 0_u64;
        let mut cycle_overflow = false;
        let mut cycle_valid = true;
        for phase_source in &source.phases {
            if let Some(first_span) = phase_keys.get(&phase_source.header.stable_key) {
                let mut diagnostic = Diagnostic::duplicate_signal_phase_key(
                    &source.header.stable_key,
                    &phase_source.header.stable_key,
                    phase_source.header.span.clone(),
                    first_span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
                continue;
            }
            phase_keys.insert(
                Arc::clone(&phase_source.header.stable_key),
                phase_source.header.span.clone(),
            );

            if phase_source.duration_ms == 0
                || phase_source.duration_ms > MAX_PORTABLE_SIGNAL_TIME_MS
            {
                cycle_valid = false;
                let mut diagnostic = Diagnostic::invalid_signal_phase_duration(
                    &source.header.stable_key,
                    &phase_source.header.stable_key,
                    phase_source.duration_ms,
                    MAX_PORTABLE_SIGNAL_TIME_MS,
                    phase_source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            } else if !cycle_overflow {
                match cycle_duration_ms.checked_add(phase_source.duration_ms) {
                    Some(sum) if sum <= MAX_PORTABLE_SIGNAL_TIME_MS => {
                        cycle_duration_ms = sum;
                    }
                    _ => {
                        cycle_overflow = true;
                        cycle_valid = false;
                        let mut diagnostic = Diagnostic::signal_cycle_duration_overflow(
                            &source.header.stable_key,
                            MAX_PORTABLE_SIGNAL_TIME_MS,
                            source.header.span.clone(),
                        );
                        diagnostic.set_canonical_module_order(module_order);
                        diagnostics.push(diagnostic);
                    }
                }
            }

            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::SignalControllerStableId,
                    controllers
                        .get(controller_key)
                        .stable_id
                        .as_untyped()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::PhaseKey,
                    phase_source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = SignalPhaseId::from_untyped(derive_identity(
                unit,
                identities,
                location.source_module_index as usize,
                EntityKind::SignalPhase,
                &phase_source.header.stable_key,
                &phase_source.header.span,
                &fields,
            )?);

            let mut states_by_position: Vec<Option<(SignalAspect, SourceLocation)>> =
                vec![None; resolved_groups.len()];
            for state in &phase_source.states {
                let Some(group_key) = resolve_reference(
                    module_lookup,
                    &group_symbols,
                    &state.signal_group,
                    EntityKind::SignalPhase,
                    &phase_source.header,
                    module_order,
                    &mut diagnostics,
                ) else {
                    continue;
                };
                let Some(&position) = group_positions.get(&group_key) else {
                    let mut diagnostic = Diagnostic::unknown_signal_phase_group(
                        &source.header.stable_key,
                        &phase_source.header.stable_key,
                        &groups.get(group_key).stable_key,
                        state.signal_group.span.clone(),
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(module_order);
                    diagnostics.push(diagnostic);
                    continue;
                };
                if let Some((_, first_span)) = &states_by_position[position] {
                    let mut diagnostic = Diagnostic::duplicate_signal_phase_group(
                        &source.header.stable_key,
                        &phase_source.header.stable_key,
                        &groups.get(group_key).stable_key,
                        state.signal_group.span.clone(),
                        first_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(module_order);
                    diagnostics.push(diagnostic);
                } else {
                    states_by_position[position] =
                        Some((state.aspect, state.signal_group.span.clone()));
                }
            }
            let state_start = phase_states.len();
            for (position, group_key) in resolved_groups.iter().copied().enumerate() {
                let Some((aspect, source_span)) = &states_by_position[position] else {
                    let mut diagnostic = Diagnostic::missing_signal_phase_group(
                        &source.header.stable_key,
                        &phase_source.header.stable_key,
                        &groups.get(group_key).stable_key,
                        phase_source.header.span.clone(),
                        groups.get(group_key).source_span.clone(),
                    );
                    diagnostic.set_canonical_module_order(module_order);
                    diagnostics.push(diagnostic);
                    // 失败路径不补虚构状态，保证阶段分配不会超过输入关系数预算。
                    continue;
                };
                phase_states.push(HirSignalPhaseState {
                    signal_group: group_key,
                    aspect: *aspect,
                    source_location: unit
                        .resolve_source_location_for_module(module_order, source_span)?,
                });
            }
            phases
                .push(HirSignalPhase {
                    module: controllers.get(controller_key).module,
                    stable_key: Arc::clone(&phase_source.header.stable_key),
                    stable_id,
                    controller: controller_key,
                    duration_ms: phase_source.duration_ms,
                    states: TableRange::try_from_usize(
                        state_start,
                        phase_states.len().saturating_sub(state_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(
                            overflow,
                            &unit.limits,
                            Some(phase_source.header.span.clone()),
                        )
                    })?,
                    controller_relation_source_location: unit.resolve_source_location_for_module(
                        module_order,
                        &phase_source.controller_relation_span,
                    )?,
                    source_span: phase_source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(
                        overflow,
                        &unit.limits,
                        Some(phase_source.header.span.clone()),
                    )
                })?;
        }
        let controller = controllers.get_mut(controller_key);
        controller.cycle_duration_ms = cycle_duration_ms;
        controller.phases =
            TableRange::try_from_usize(phase_start, phases.len().saturating_sub(phase_start))
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
        if !source.phases.is_empty()
            && cycle_valid
            && (source.offset_ms > MAX_PORTABLE_SIGNAL_TIME_MS
                || source.offset_ms >= cycle_duration_ms)
        {
            let mut diagnostic = Diagnostic::invalid_signal_controller_offset(
                &source.header.stable_key,
                source.offset_ms,
                cycle_duration_ms,
                MAX_PORTABLE_SIGNAL_TIME_MS,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
        }
    }

    for location in &group_sources {
        if owners[location.hir_key.index()].is_none() {
            let group = groups.get(location.hir_key);
            let mut diagnostic =
                Diagnostic::unowned_signal_group(&group.stable_key, group.source_span.clone());
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
    }

    // 正向门绑定完成后，按组/门稳定身份建立连续反向表；运行时不需要扫描全部门。
    let mut usages = Vec::<(HirSignalGroupKey, HirManeuverGateKey)>::with_capacity(count_to_usize(
        counts.controlled_gates,
        &unit.limits,
    )?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let module_order = u32::try_from(module_index).unwrap_or(u32::MAX);
        let mut declarations: Vec<_> = source_module
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                TypedAstDeclaration::ManeuverGate(gate) => Some(gate),
                _ => None,
            })
            .collect();
        declarations.sort_unstable_by(|left, right| {
            left.header.source_address.cmp(&right.header.source_address)
        });
        for source in declarations {
            let gate_key = gate_symbols
                .get(module_key, &source.header.source_address)
                .expect("control HIR must contain every ManeuverGate symbol");
            match &source.signal_control {
                OwnedSignalControl::None => {}
                OwnedSignalControl::Group(reference) => {
                    let Some(group_key) = resolve_reference(
                        module_lookup,
                        &group_symbols,
                        reference,
                        EntityKind::ManeuverGate,
                        &source.header,
                        module_order,
                        &mut diagnostics,
                    ) else {
                        continue;
                    };
                    maneuver_gates[gate_key.index()].signal_control = HirSignalControl::Group {
                        signal_group: group_key,
                        source_location: unit
                            .resolve_source_location_for_module(module_order, &reference.span)?,
                    };
                    usages.push((group_key, gate_key));
                }
            }
        }
    }
    usages.sort_unstable_by(|left, right| {
        (
            groups.get(left.0).stable_id,
            maneuver_gates[left.1.index()].stable_id,
        )
            .cmp(&(
                groups.get(right.0).stable_id,
                maneuver_gates[right.1.index()].stable_id,
            ))
    });
    // `usages` 按 StableId 排序，不能再假设其 owner 顺序与 arena key 相同。先从实际
    // 连续分组回填每个 group 的 range，避免 arena 顺序与 StableId 顺序不一致时把
    // 相邻 group 的成员切片错配给当前 group。
    let mut usage_ranges = vec![(0_usize, 0_usize); groups.len()];
    let mut usage_cursor = 0_usize;
    while usage_cursor < usages.len() {
        let group = usages[usage_cursor].0;
        let start = usage_cursor;
        while usage_cursor < usages.len() && usages[usage_cursor].0 == group {
            usage_cursor = usage_cursor.saturating_add(1);
        }
        usage_ranges[group.index()] = (start, usage_cursor.saturating_sub(start));
    }
    for (index, (start, count)) in usage_ranges.iter().copied().enumerate() {
        let group_key = HirSignalGroupKey::from_raw(
            u32::try_from(index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        groups.get_mut(group_key).maneuver_gates = TableRange::try_from_usize(start, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        if count == 0 {
            let group = groups.get(group_key);
            let mut diagnostic =
                Diagnostic::unused_signal_group(&group.stable_key, group.source_span.clone());
            diagnostic.set_canonical_module_order(group.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    Ok(SignalHir {
        signal_groups: groups.into_boxed_slice(),
        signal_controllers: controllers.into_boxed_slice(),
        signal_controller_groups: controller_group_rows.into_boxed_slice(),
        signal_phases: phases.into_boxed_slice(),
        signal_phase_states: phase_states.into_boxed_slice(),
        signal_group_maneuver_gates: usages
            .into_iter()
            .map(|(_, maneuver_gate)| HirSignalGroupManeuverGate { maneuver_gate })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}
