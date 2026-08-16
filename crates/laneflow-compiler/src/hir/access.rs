//! 准入领域 HIR：参与者类别、车辆配置与准入规则的记录、构建与验证。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    AccessEffect, AccessRuleId, EntityKind, FieldTag, ParticipantClassId, VehicleProfileId,
};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{OwnedAccessRegulation, OwnedAccessRuleTarget, TypedAstDeclaration};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{IdentityFieldInput, IdentityRegistry};
use crate::{
    AccessCapability, AccessPlane, AccessRegulationField, CompilationUnit, CompileLimitDimension,
    Diagnostic, DiagnosticBundle, SourceLocation,
};

use super::{
    AccessCounts, CanonicalDeclarationSource, CrossSectionHir, HirAccessRuleKey, HirAccessRuleTag,
    HirFacilityBandKey, HirLaneEdge, HirLaneEdgeKey, HirLaneEdgeTag, HirLaneGroupKey,
    HirManeuverPath, HirManeuverPathKey, HirModuleKey, HirParticipantClassKey,
    HirParticipantClassTag, HirRoadSectionKey, HirVehicleProfileTag, SymbolTable, arena_overflow,
    count_to_usize, declaration_header, derive_identity, resolve_reference,
};

/// 已解析父类并编译单继承层级信息的参与者类别。
#[derive(Debug, PartialEq)]
pub(crate) struct HirParticipantClass {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: ParticipantClassId,
    pub(crate) parent: Option<HirParticipantClassKey>,
    pub(crate) parent_source_span: Option<SourceLocation>,
    /// 根类别为 0；准入规则以更深类别作为更高 specificity。
    pub(crate) depth: u32,
    /// Euler tour 半开子树区间 `[enter, exit)`。
    pub(crate) subtree_enter: u32,
    pub(crate) subtree_exit: u32,
    pub(crate) source_span: SourceLocation,
}

/// 已解析唯一参与者类别、并保持 current Core IIDM `f64` 语义的车辆配置。
#[derive(Debug, PartialEq)]
pub(crate) struct HirVehicleProfile {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: VehicleProfileId,
    pub(crate) participant_class: HirParticipantClassKey,
    pub(crate) participant_class_source_span: SourceLocation,
    pub(crate) length_meters: f64,
    pub(crate) desired_speed_meters_per_second: f64,
    pub(crate) min_gap_meters: f64,
    pub(crate) time_headway_seconds: f64,
    pub(crate) max_acceleration_meters_per_second_squared: f64,
    pub(crate) comfortable_deceleration_meters_per_second_squared: f64,
    pub(crate) emergency_deceleration_meters_per_second_squared: f64,
    pub(crate) source_span: SourceLocation,
}

/// HIR 中已解析且保持求值平面边界的准入目标。
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum HirAccessTarget {
    LaneEdge(HirLaneEdgeKey),
    LaneGroup(HirLaneGroupKey),
    RoadSection(HirRoadSectionKey),
    ManeuverPath(HirManeuverPathKey),
}

/// 已验证的法规来源信息；该值参与规范 LIR，但不参与准入组合键。
#[derive(Debug, PartialEq)]
pub(crate) struct HirAccessRegulation {
    pub(crate) jurisdiction: Arc<str>,
    pub(crate) version: Arc<str>,
    pub(crate) source: Option<Arc<str>>,
}

/// 一条准入规则引用的参与者类别。
#[derive(Debug, PartialEq)]
pub(crate) struct HirAccessRuleParticipantClass {
    pub(crate) participant_class: HirParticipantClassKey,
    pub(crate) source_span: SourceLocation,
}

/// 完成静态引用解析和组合歧义验证的准入规则。
#[derive(Debug, PartialEq)]
pub(crate) struct HirAccessRule {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AccessRuleId,
    pub(crate) target: HirAccessTarget,
    pub(crate) target_source_span: SourceLocation,
    pub(crate) effect: AccessEffect,
    pub(crate) participant_classes: TableRange<HirAccessRuleParticipantClass>,
    pub(crate) regulation: Option<HirAccessRegulation>,
    pub(crate) priority: i32,
    pub(crate) source_span: SourceLocation,
}

#[derive(Default)]
pub(crate) struct AccessHir {
    pub(crate) participant_classes: Box<[HirParticipantClass]>,
    pub(crate) vehicle_profiles: Box<[HirVehicleProfile]>,
    pub(crate) access_rules: Box<[HirAccessRule]>,
    pub(crate) access_rule_participant_classes: Box<[HirAccessRuleParticipantClass]>,
}

#[derive(Clone, Copy)]
pub(crate) struct AccessCandidate {
    plane: AccessPlane,
    target_kind: EntityKind,
    target_index: u32,
    participant_class: HirParticipantClassKey,
    priority: i32,
    effect: AccessEffect,
    rule: HirAccessRuleKey,
}

/// 同一编译单元内首条有效法规来源，用于给后续不一致规则提供稳定的对照与关联位置。
struct FirstAccessRegulation {
    jurisdiction: Arc<str>,
    version: Arc<str>,
    rule_key: Arc<str>,
    source_span: SourceLocation,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(crate) fn build_access_hir(
    unit: &CompilationUnit,
    counts: &AccessCounts,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    cross_section: &CrossSectionHir,
    maneuver_paths: &[HirManeuverPath],
    identities: &mut IdentityRegistry,
) -> Result<AccessHir, DiagnosticBundle> {
    if counts.entity_count() == 0 {
        return Ok(AccessHir::default());
    }

    let mut class_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::ParticipantClass(_)))
            .count()
    }));
    let mut edge_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::LaneEdge(_)))
            .count()
    }));
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::LaneGroup(_)))
            .count()
    }));
    let mut section_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::RoadSection(_)))
            .count()
    }));
    let mut path_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::ManeuverPath(_)))
            .count()
    }));
    let mut band_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|item| matches!(item, TypedAstDeclaration::FacilityBand(_)))
            .count()
    }));
    for (key, edge) in lane_edges.iter() {
        edge_symbols.insert(edge.module, edge.source_address.clone(), key);
    }
    for (index, group) in cross_section.lane_groups.iter().enumerate() {
        group_symbols.insert(
            group.module,
            group.source_address.clone(),
            HirLaneGroupKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }
    for (index, section) in cross_section.road_sections.iter().enumerate() {
        section_symbols.insert(
            section.module,
            section.source_address.clone(),
            HirRoadSectionKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }
    for (index, path) in maneuver_paths.iter().enumerate() {
        path_symbols.insert(
            path.module,
            path.source_address.clone(),
            HirManeuverPathKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }
    for (index, band) in cross_section.facility_bands.iter().enumerate() {
        band_symbols.insert(
            band.module,
            band.source_address.clone(),
            HirFacilityBandKey::from_raw(u32::try_from(index).expect("HIR table is u32-bounded")),
        );
    }

    let mut classes = TypedArena::<HirParticipantClassTag, HirParticipantClass>::with_capacity(
        count_to_usize(counts.participant_classes, &unit.limits)?,
    );
    let mut class_sources =
        Vec::with_capacity(count_to_usize(counts.participant_classes, &unit.limits)?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("module table is u32-bounded"),
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::ParticipantClass(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::ParticipantClass(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("filtered declaration must be ParticipantClass");
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::ParticipantClassKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = ParticipantClassId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::ParticipantClass,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = classes
                .push(HirParticipantClass {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    parent: None,
                    parent_source_span: None,
                    depth: 0,
                    subtree_enter: 0,
                    subtree_exit: 0,
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            class_symbols.insert(module_key, source.header.source_address.clone(), key);
            class_sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index)
                    .expect("module table is u32-bounded"),
                declaration_index: u32::try_from(declaration_index)
                    .expect("declaration table is u32-bounded"),
                hir_key: key,
            });
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for location in &class_sources {
        let module_index = usize::try_from(location.source_module_index)
            .expect("u32 module index must fit usize on supported targets");
        let declaration_index = usize::try_from(location.declaration_index)
            .expect("u32 declaration index must fit usize on supported targets");
        let TypedAstDeclaration::ParticipantClass(source) =
            &unit.modules[module_index].declarations[declaration_index]
        else {
            unreachable!("canonical class source must still name ParticipantClass");
        };
        if let Some(parent) = &source.extends {
            classes.get_mut(location.hir_key).parent = resolve_reference(
                module_lookup,
                &class_symbols,
                parent,
                EntityKind::ParticipantClass,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            );
            classes.get_mut(location.hir_key).parent_source_span = Some(parent.span.clone());
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 单继承链用三色迭代遍历检测，避免极深分类法消耗线程栈。
    let mut state = vec![0_u8; classes.len()];
    for start_index in 0..classes.len() {
        if state[start_index] != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut cursor = HirParticipantClassKey::from_raw(
            u32::try_from(start_index).expect("HIR table is u32-bounded"),
        );
        let mut cycle_cursor = None;
        while state[cursor.index()] == 0 {
            state[cursor.index()] = 1;
            path.push(cursor);
            let Some(parent) = classes.get(cursor).parent else {
                break;
            };
            cursor = parent;
        }
        // 无父类的根节点也处于本轮的 visiting 状态；只有沿 parent 边重新进入
        // visiting 节点才构成环，不能仅凭最终状态判断。
        if classes
            .get(*path.last().expect("fresh traversal is non-empty"))
            .parent
            .is_some()
            && state[cursor.index()] == 1
        {
            cycle_cursor = Some(cursor);
        }
        if let Some(cursor) = cycle_cursor {
            let cycle_start = path.iter().position(|key| *key == cursor).unwrap_or(0);
            let cycle = &path[cycle_start..];
            let representative = cycle
                .iter()
                .copied()
                .min_by(|left, right| {
                    classes
                        .get(*left)
                        .stable_key
                        .cmp(&classes.get(*right).stable_key)
                })
                .expect("active traversal contains its cycle cursor");
            let related_spans = cycle
                .iter()
                .filter(|key| **key != representative)
                .map(|key| classes.get(*key).source_span.clone())
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let mut diagnostic = Diagnostic::participant_class_inheritance_cycle(
                &classes.get(representative).stable_key,
                classes.get(representative).source_span.clone(),
                related_spans,
            );
            diagnostic.set_canonical_module_order(classes.get(representative).module.raw());
            diagnostics.push(diagnostic);
        }
        for key in path.into_iter().rev() {
            if classes
                .get(key)
                .parent
                .is_none_or(|parent| state[parent.index()] == 2)
            {
                classes.get_mut(key).depth = classes
                    .get(key)
                    .parent
                    .map_or(0, |parent| classes.get(parent).depth.saturating_add(1));
            }
            state[key.index()] = 2;
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // first-child/next-sibling 数组让层级闭包只需连续内存，不为每个类别单独分配 Vec。
    let mut first_child = vec![None; classes.len()];
    let mut next_sibling = vec![None; classes.len()];
    let mut first_root = None;
    for (index, sibling_slot) in next_sibling.iter_mut().enumerate() {
        let key = HirParticipantClassKey::from_raw(
            u32::try_from(index).expect("HIR table is u32-bounded"),
        );
        if let Some(parent) = classes.get(key).parent {
            *sibling_slot = first_child[parent.index()];
            first_child[parent.index()] = Some(key);
        } else {
            *sibling_slot = first_root;
            first_root = Some(key);
        }
    }
    let mut stack = Vec::with_capacity(classes.len().saturating_mul(2));
    let mut root = first_root;
    while let Some(key) = root {
        stack.push((key, false));
        root = next_sibling[key.index()];
    }
    let mut euler = 0_u32;
    while let Some((key, exiting)) = stack.pop() {
        if exiting {
            classes.get_mut(key).subtree_exit = euler;
            continue;
        }
        classes.get_mut(key).subtree_enter = euler;
        euler = euler.checked_add(1).ok_or_else(|| {
            arena_overflow(
                ArenaKeyOverflow,
                &unit.limits,
                Some(classes.get(key).source_span.clone()),
            )
        })?;
        stack.push((key, true));
        let mut child = first_child[key.index()];
        while let Some(child_key) = child {
            stack.push((child_key, false));
            child = next_sibling[child_key.index()];
        }
    }

    // VehicleProfile 只消费已经闭合的分类法；它不会反向改变类别层级或把车辆参数
    // 提升为跨执行域能力。先登记规范身份，再统一解析类别，保留前向/跨模块引用。
    let mut profiles = TypedArena::<HirVehicleProfileTag, HirVehicleProfile>::with_capacity(
        count_to_usize(counts.vehicle_profiles, &unit.limits)?,
    );
    let mut profile_sources =
        Vec::with_capacity(count_to_usize(counts.vehicle_profiles, &unit.limits)?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("module table is u32-bounded"),
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::VehicleProfile(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::VehicleProfile(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("filtered declaration must be VehicleProfile");
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::VehicleProfileKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = VehicleProfileId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::VehicleProfile,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let iidm = source.iidm;
            let key = profiles
                .push(HirVehicleProfile {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    participant_class: HirParticipantClassKey::from_raw(0),
                    participant_class_source_span: source.participant_class.span.clone(),
                    length_meters: iidm.length_meters,
                    desired_speed_meters_per_second: iidm.desired_speed_meters_per_second,
                    min_gap_meters: iidm.min_gap_meters,
                    time_headway_seconds: iidm.time_headway_seconds,
                    max_acceleration_meters_per_second_squared: iidm
                        .max_acceleration_meters_per_second_squared,
                    comfortable_deceleration_meters_per_second_squared: iidm
                        .comfortable_deceleration_meters_per_second_squared,
                    emergency_deceleration_meters_per_second_squared: iidm
                        .emergency_deceleration_meters_per_second_squared,
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            profile_sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index)
                    .expect("module table is u32-bounded"),
                declaration_index: u32::try_from(declaration_index)
                    .expect("declaration table is u32-bounded"),
                hir_key: key,
            });
        }
    }
    for location in &profile_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let TypedAstDeclaration::VehicleProfile(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical vehicle profile source changed kind");
        };
        if let Some(participant_class) = resolve_reference(
            module_lookup,
            &class_symbols,
            &source.participant_class,
            EntityKind::VehicleProfile,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) {
            profiles.get_mut(location.hir_key).participant_class = participant_class;
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut rules = TypedArena::<HirAccessRuleTag, HirAccessRule>::with_capacity(count_to_usize(
        counts.access_rules,
        &unit.limits,
    )?);
    let mut rule_sources = Vec::with_capacity(count_to_usize(counts.access_rules, &unit.limits)?);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index).expect("module table is u32-bounded"),
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                matches!(declaration, TypedAstDeclaration::AccessRule(_)).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by_key(|index| {
            &declaration_header(&source_module.declarations[*index]).source_address
        });
        for declaration_index in declaration_indices {
            let TypedAstDeclaration::AccessRule(source) =
                &source_module.declarations[declaration_index]
            else {
                unreachable!("filtered declaration must be AccessRule");
            };
            let fields = [
                IdentityFieldInput::new(
                    FieldTag::AuthoringNamespaceId,
                    source_module
                        .descriptor()
                        .authoring_namespace_id()
                        .as_bytes(),
                ),
                IdentityFieldInput::new(
                    FieldTag::AccessRuleKey,
                    source.header.stable_key.as_bytes(),
                ),
            ];
            let stable_id = AccessRuleId::from_untyped(derive_identity(
                unit,
                identities,
                module_index,
                EntityKind::AccessRule,
                &source.header.stable_key,
                &source.header.span,
                &fields,
            )?);
            let key = rules
                .push(HirAccessRule {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id,
                    // 仅作首遍占位；目标解析失败时整个 HIR 不会提交。
                    target: HirAccessTarget::LaneEdge(HirLaneEdgeKey::from_raw(0)),
                    target_source_span: source.header.span.clone(),
                    effect: source.effect,
                    participant_classes: TableRange::empty(),
                    regulation: None,
                    priority: source.priority,
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            rule_sources.push(CanonicalDeclarationSource {
                source_module_index: u32::try_from(module_index)
                    .expect("module table is u32-bounded"),
                declaration_index: u32::try_from(declaration_index)
                    .expect("declaration table is u32-bounded"),
                hir_key: key,
            });
        }
    }

    let mut rule_classes =
        Vec::with_capacity(count_to_usize(counts.rule_class_references, &unit.limits)?);
    let mut first_regulation: Option<FirstAccessRegulation> = None;
    for location in &rule_sources {
        let module_index = usize::try_from(location.source_module_index)
            .expect("u32 module index must fit usize on supported targets");
        let declaration_index = usize::try_from(location.declaration_index)
            .expect("u32 declaration index must fit usize on supported targets");
        let TypedAstDeclaration::AccessRule(source) =
            &unit.modules[module_index].declarations[declaration_index]
        else {
            unreachable!("canonical rule source must still name AccessRule");
        };
        let target = resolve_access_target(
            module_lookup,
            &edge_symbols,
            &group_symbols,
            &section_symbols,
            &path_symbols,
            &band_symbols,
            source,
            location.source_module_index,
            &mut diagnostics,
        );
        if let Some(target) = target {
            rules.get_mut(location.hir_key).target = target;
            rules.get_mut(location.hir_key).target_source_span = access_target_source_span(source);
        }

        if source.participant_classes.is_empty() {
            let mut diagnostic = Diagnostic::empty_access_rule_participant_classes(
                &source.header.stable_key,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(location.source_module_index);
            diagnostics.push(diagnostic);
        }
        let start = rule_classes.len();
        let mut resolved_classes = Vec::with_capacity(source.participant_classes.len());
        for reference in &source.participant_classes {
            if let Some(participant_class) = resolve_reference(
                module_lookup,
                &class_symbols,
                reference,
                EntityKind::AccessRule,
                &source.header,
                location.source_module_index,
                &mut diagnostics,
            ) {
                resolved_classes.push((participant_class, reference.span.clone()));
            }
        }
        resolved_classes.sort_unstable_by_key(|(participant_class, _)| *participant_class);
        resolved_classes.dedup_by_key(|(participant_class, _)| *participant_class);
        rule_classes.extend(resolved_classes.into_iter().map(
            |(participant_class, source_span)| HirAccessRuleParticipantClass {
                participant_class,
                source_span,
            },
        ));
        rules.get_mut(location.hir_key).participant_classes =
            TableRange::try_from_usize(start, rule_classes.len().saturating_sub(start)).map_err(
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;

        if let Some(regulation) = &source.regulation {
            let valid = validate_access_regulation(
                regulation,
                source,
                location.source_module_index,
                &mut diagnostics,
            );
            if valid {
                if let Some(first) = &first_regulation {
                    if first.jurisdiction.as_ref() != regulation.jurisdiction.as_ref()
                        || first.version.as_ref() != regulation.version.as_ref()
                    {
                        let mut diagnostic = Diagnostic::access_regulation_mismatch(
                            &first.rule_key,
                            &first.jurisdiction,
                            &first.version,
                            &source.header.stable_key,
                            &regulation.jurisdiction,
                            &regulation.version,
                            source.header.span.clone(),
                            first.source_span.clone(),
                        );
                        diagnostic.set_canonical_module_order(location.source_module_index);
                        diagnostics.push(diagnostic);
                    }
                } else {
                    first_regulation = Some(FirstAccessRegulation {
                        jurisdiction: Arc::clone(&regulation.jurisdiction),
                        version: Arc::clone(&regulation.version),
                        rule_key: Arc::clone(&source.header.stable_key),
                        source_span: source.header.span.clone(),
                    });
                }
                rules.get_mut(location.hir_key).regulation = Some(HirAccessRegulation {
                    jurisdiction: Arc::clone(&regulation.jurisdiction),
                    version: Arc::clone(&regulation.version),
                    source: regulation.source.clone(),
                });
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    validate_access_ambiguity(
        unit,
        counts,
        lane_edges,
        cross_section,
        maneuver_paths,
        &classes,
        &rules,
        &rule_classes,
        &mut diagnostics,
    )?;
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    Ok(AccessHir {
        participant_classes: classes.into_boxed_slice(),
        vehicle_profiles: profiles.into_boxed_slice(),
        access_rules: rules.into_boxed_slice(),
        access_rule_participant_classes: rule_classes.into_boxed_slice(),
    })
}

#[allow(clippy::too_many_arguments)]
fn resolve_access_target(
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    group_symbols: &SymbolTable<HirLaneGroupKey>,
    section_symbols: &SymbolTable<HirRoadSectionKey>,
    path_symbols: &SymbolTable<HirManeuverPathKey>,
    band_symbols: &SymbolTable<HirFacilityBandKey>,
    source: &crate::declaration::AccessRuleDeclaration,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> Option<HirAccessTarget> {
    match &source.target {
        OwnedAccessRuleTarget::LaneEdge(reference) => resolve_reference(
            module_lookup,
            edge_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::LaneEdge),
        OwnedAccessRuleTarget::LaneGroup(reference) => resolve_reference(
            module_lookup,
            group_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::LaneGroup),
        OwnedAccessRuleTarget::RoadSection(reference) => resolve_reference(
            module_lookup,
            section_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::RoadSection),
        OwnedAccessRuleTarget::ManeuverPath(reference) => resolve_reference(
            module_lookup,
            path_symbols,
            reference,
            EntityKind::AccessRule,
            &source.header,
            module_order,
            diagnostics,
        )
        .map(HirAccessTarget::ManeuverPath),
        OwnedAccessRuleTarget::FacilityBand(reference) => {
            if resolve_reference(
                module_lookup,
                band_symbols,
                reference,
                EntityKind::AccessRule,
                &source.header,
                module_order,
                diagnostics,
            )
            .is_some()
            {
                let mut diagnostic = Diagnostic::access_capability_unavailable(
                    &source.header.stable_key,
                    AccessCapability::FacilityBandTarget,
                    reference.span.clone(),
                );
                diagnostic.set_canonical_module_order(module_order);
                diagnostics.push(diagnostic);
            }
            None
        }
    }
}

fn access_target_source_span(source: &crate::declaration::AccessRuleDeclaration) -> SourceLocation {
    match &source.target {
        OwnedAccessRuleTarget::LaneEdge(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::LaneGroup(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::RoadSection(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::ManeuverPath(reference) => reference.span.clone(),
        OwnedAccessRuleTarget::FacilityBand(reference) => reference.span.clone(),
    }
}

fn validate_access_regulation(
    regulation: &OwnedAccessRegulation,
    source: &crate::declaration::AccessRuleDeclaration,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> bool {
    let mut valid = true;
    for (field, value) in [
        (
            AccessRegulationField::Jurisdiction,
            regulation.jurisdiction.as_ref(),
        ),
        (AccessRegulationField::Version, regulation.version.as_ref()),
    ] {
        let character_count = u32::try_from(value.chars().count()).unwrap_or(u32::MAX);
        if !(1..=128).contains(&character_count) {
            let mut diagnostic = Diagnostic::invalid_access_regulation_string(
                &source.header.stable_key,
                field,
                character_count,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
            valid = false;
        }
    }
    if let Some(value) = &regulation.source {
        let character_count = u32::try_from(value.chars().count()).unwrap_or(u32::MAX);
        if !(1..=128).contains(&character_count) {
            let mut diagnostic = Diagnostic::invalid_access_regulation_string(
                &source.header.stable_key,
                AccessRegulationField::Source,
                character_count,
                source.header.span.clone(),
            );
            diagnostic.set_canonical_module_order(module_order);
            diagnostics.push(diagnostic);
            valid = false;
        }
    }
    valid
}

#[allow(clippy::too_many_arguments)]
fn validate_access_ambiguity(
    unit: &CompilationUnit,
    counts: &AccessCounts,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    cross_section: &CrossSectionHir,
    maneuver_paths: &[HirManeuverPath],
    classes: &TypedArena<HirParticipantClassTag, HirParticipantClass>,
    rules: &TypedArena<HirAccessRuleTag, HirAccessRule>,
    rule_classes: &[HirAccessRuleParticipantClass],
    diagnostics: &mut DiagnosticCollector,
) -> Result<(), DiagnosticBundle> {
    let mut candidates =
        Vec::with_capacity(count_to_usize(counts.rule_class_references, &unit.limits)?);
    for (rule_key, rule) in rules.iter() {
        let (plane, target_kind, target_index) = match rule.target {
            HirAccessTarget::LaneEdge(target) => {
                (AccessPlane::Edge, EntityKind::LaneEdge, target.raw())
            }
            HirAccessTarget::LaneGroup(target) => {
                (AccessPlane::Edge, EntityKind::LaneGroup, target.raw())
            }
            HirAccessTarget::RoadSection(target) => {
                (AccessPlane::Edge, EntityKind::RoadSection, target.raw())
            }
            HirAccessTarget::ManeuverPath(target) => (
                AccessPlane::ManeuverPath,
                EntityKind::ManeuverPath,
                target.raw(),
            ),
        };
        // 单继承意味着同深度且相交的类别子树必有相同根；完整横断面所有者树又保证
        // 同 specificity 的两个不同 edge/group/section target 不会覆盖同一边。因此
        // 只需比较规则实际声明的 target 与 selector，无需展开全部边和全部后代类别。
        for selector in &rule_classes[rule.participant_classes.as_usize_range()] {
            candidates.push(AccessCandidate {
                plane,
                target_kind,
                target_index,
                participant_class: selector.participant_class,
                priority: rule.priority,
                effect: rule.effect,
                rule: rule_key,
            });
        }
    }
    candidates.sort_unstable_by(|left, right| {
        (
            left.plane,
            left.target_kind,
            left.target_index,
            left.participant_class,
            left.priority,
        )
            .cmp(&(
                right.plane,
                right.target_kind,
                right.target_index,
                right.participant_class,
                right.priority,
            ))
            .then_with(|| left.rule.cmp(&right.rule))
    });

    let mut cursor = 0;
    while cursor < candidates.len() {
        let first = candidates[cursor];
        let group_key = (
            first.plane,
            first.target_kind,
            first.target_index,
            first.participant_class,
            first.priority,
        );
        let mut allow = None;
        let mut deny = None;
        while cursor < candidates.len()
            && (
                candidates[cursor].plane,
                candidates[cursor].target_kind,
                candidates[cursor].target_index,
                candidates[cursor].participant_class,
                candidates[cursor].priority,
            ) == group_key
        {
            match candidates[cursor].effect {
                AccessEffect::Allow => {
                    allow.get_or_insert(candidates[cursor].rule);
                }
                AccessEffect::Deny => {
                    deny.get_or_insert(candidates[cursor].rule);
                }
                _ => unreachable!("AccessEffect extension requires compiler update"),
            }
            cursor += 1;
        }
        if let (Some(allow_rule), Some(deny_rule)) = (allow, deny) {
            let allow_rule = rules.get(allow_rule);
            let deny_rule = rules.get(deny_rule);
            let participant_class = classes.get(first.participant_class);
            let target_key = match first.target_kind {
                EntityKind::LaneEdge => lane_edges
                    .get(HirLaneEdgeKey::from_raw(first.target_index))
                    .stable_key
                    .as_ref(),
                EntityKind::LaneGroup => cross_section.lane_groups
                    [usize::try_from(first.target_index).expect("u32 group index must fit usize")]
                .stable_key
                .as_ref(),
                EntityKind::RoadSection => {
                    cross_section.road_sections[usize::try_from(first.target_index)
                        .expect("u32 section index must fit usize")]
                    .stable_key
                    .as_ref()
                }
                EntityKind::ManeuverPath => maneuver_paths
                    [usize::try_from(first.target_index).expect("u32 path index must fit usize")]
                .stable_key
                .as_ref(),
                _ => unreachable!("access candidate target kinds are closed"),
            };
            let mut diagnostic = Diagnostic::access_rule_ambiguity(
                first.plane,
                first.target_kind,
                target_key,
                &participant_class.stable_key,
                &allow_rule.stable_key,
                &deny_rule.stable_key,
                deny_rule.source_span.clone(),
                allow_rule.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(deny_rule.module.raw());
            diagnostics.push(diagnostic);
        }
        while cursor < candidates.len()
            && (
                candidates[cursor].plane,
                candidates[cursor].target_kind,
                candidates[cursor].target_index,
                candidates[cursor].participant_class,
                candidates[cursor].priority,
            ) == group_key
        {
            cursor += 1;
        }
    }
    Ok(())
}
