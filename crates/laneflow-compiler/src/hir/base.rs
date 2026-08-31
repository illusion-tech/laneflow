//! HIR 基础构造（base）：模块轴、导入、车道图边键分配与后继引用解析。
//!
//! [`HirBase::build`] 整体承接拆分前 `build_hir` 的 base 段：先冻结规范模块轴与平坦导入表，
//! 再按 `(canonical module order, stable key)` 为全部 LaneEdge 分配键并建立符号表，最后统一
//! 解析已规范化的后继引用。产物对下游八个领域只读；仅有的两条跨领域写边
//! （control→junction、signal→control）不触及本结构的表。可失败调用点（容量换算、
//! arena push、identity 注册、`u32` 与 `TableRange` 换算）与拆分前的位置、顺序逐点一致；
//! `IdentityRegistry` 在本函数内按原序创建并随产物交还编排层，供全部领域共享后于装配前
//! 丢弃。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{EntityKind, FieldTag, LaneEdgeId, StableId128};

use crate::arena::{ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{OwnedEntityReference, TypedAstDeclaration, TypedAstEntityAddress};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{
    IdentityFieldInput, IdentityRegistrationError, IdentityRegistry, encode_canonical_identity,
};
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceLocation};

use super::plan::HirBuildPlan;
use super::{
    HirLaneEdgeKey, HirLaneEdgeTag, HirModuleKey, HirModuleTag, HirRoadCorridor,
    HirRoadCorridorKey, HirRoadCorridorTag, arena_overflow, count_to_usize, lane_edge_declaration,
};

/// 已解析为 HIR 模块键的显式导入边。
#[derive(Debug, PartialEq)]
pub(crate) struct HirImport {
    /// 被导入模块；目标在规范模块顺序中位于当前模块之前。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) target: HirModuleKey,
    /// 原始导入声明位置。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) source_span: SourceLocation,
}

/// HIR 模块记录及其在平坦导入表中的连续区间。
#[derive(Debug, PartialEq)]
pub(crate) struct HirModule {
    /// 声明身份与跨模块解析使用的稳定命名空间。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 此模块在 `HirUnit::imports` 中的半开区间。
    pub(crate) imports: TableRange<HirImport>,
    /// 模块声明位置。
    pub(crate) source_span: SourceLocation,
}

/// 已解析为 HIR 车道图边键的下游引用。
#[derive(Debug, PartialEq)]
pub(crate) struct HirLaneEdgeReference {
    /// 当前 `HirUnit::lane_edges` 中的目标键。
    pub(crate) target: HirLaneEdgeKey,
    /// 原始引用位置。
    pub(crate) source_span: SourceLocation,
}

/// 完成模块归属和下游符号解析的车道图边 HIR 记录。
#[derive(Debug, PartialEq)]
pub(crate) struct HirLaneEdge {
    /// 拥有此声明的 HIR 模块。
    pub(crate) module: HirModuleKey,
    /// 模块内稳定键；不是 HIR 致密下标。
    pub(crate) stable_key: Arc<str>,
    pub(crate) source_address: TypedAstEntityAddress,
    /// 由 `(authoringNamespaceId, laneEdgeKey)` 的完整 Identity v1 前像派生。
    pub(crate) stable_id: LaneEdgeId,
    /// 交通权威长度，单位为毫米。
    pub(crate) length_mm: u32,
    /// 基础道路限速，单位为毫米每秒。
    pub(crate) speed_limit_mm_s: u32,
    /// 此边在 `HirUnit::lane_edge_references` 中的连续下游引用区间。
    pub(crate) successors: TableRange<HirLaneEdgeReference>,
    /// 原始声明位置。
    pub(crate) source_span: SourceLocation,
}

/// 按 HIR 模块隔离的有类型符号查找索引；不提供规范遍历能力。
pub(super) struct SymbolTable<K> {
    by_module: Vec<HashMap<TypedAstEntityAddress, K>>,
}

impl<K: Copy> SymbolTable<K> {
    pub(super) fn new(module_declaration_counts: impl IntoIterator<Item = usize>) -> Self {
        Self {
            by_module: module_declaration_counts
                .into_iter()
                .map(HashMap::with_capacity)
                .collect(),
        }
    }

    pub(super) fn insert(
        &mut self,
        module: HirModuleKey,
        source_address: TypedAstEntityAddress,
        key: K,
    ) {
        let previous = self.by_module[module.index()].insert(source_address, key);
        debug_assert!(
            previous.is_none(),
            "Typed AST rejected duplicate declarations"
        );
    }

    pub(super) fn get(
        &self,
        module: HirModuleKey,
        source_address: &TypedAstEntityAddress,
    ) -> Option<K> {
        self.by_module[module.index()].get(source_address).copied()
    }
}

#[derive(Clone, Copy)]
/// 把规范 HIR 键映回 Typed AST 物理位置的阶段暂存记录。
///
/// HIR 键不能冒充来源模块/声明下标；显式保存两者可在声明排序后仍准确读取来源记录。
pub(super) struct CanonicalLaneEdgeSource {
    pub(super) source_module_index: u32,
    pub(super) declaration_index: u32,
    pub(super) hir_key: HirLaneEdgeKey,
}

/// HIR 基础构造（base）产物：模块轴、导入、车道图边表、后继引用与只读解析索引。
///
/// 全部下游领域通过字段引用只读本结构；`HirUnit` 表布局与 persistent 字节组成不变，
/// 装配时由 `HirParts::finish` 消费本结构并转为平坦 boxed 表。
pub(super) struct HirBase {
    pub(super) modules: TypedArena<HirModuleTag, HirModule>,
    pub(super) imports: Vec<HirImport>,
    pub(super) lane_edges: TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    pub(super) lane_edge_references: Vec<HirLaneEdgeReference>,
    pub(super) module_lookup: HashMap<Arc<str>, HirModuleKey>,
    pub(super) lane_edge_symbols: SymbolTable<HirLaneEdgeKey>,
}

impl HirBase {
    /// 构建基础构造产物，并交还按原序创建的 `IdentityRegistry` 供全部领域共享。
    ///
    /// 函数体内的可失败调用点与拆分前 `build_hir` base 段的位置、顺序逐点一致。
    pub(super) fn build(
        unit: &CompilationUnit,
        plan: &HirBuildPlan,
    ) -> Result<(HirBase, IdentityRegistry), DiagnosticBundle> {
        let primary_span = unit
            .modules
            .first()
            .map(|module| module.declaration_span().clone());
        let module_capacity = unit.modules.len();
        let import_capacity = count_to_usize(unit.import_edge_count, &unit.limits)?;
        let lane_edge_capacity = count_to_usize(plan.lane_edge_count, &unit.limits)?;
        let reference_capacity = count_to_usize(plan.lane_edge_reference_count, &unit.limits)?;
        // 第一阶段冻结模块键。CompilationUnit 已按依赖优先排序，因此 raw key 顺序可直接
        // 作为后续规范模块轴；module_lookup 只用于解析，不参与任何输出遍历。
        let mut modules = TypedArena::<HirModuleTag, HirModule>::with_capacity(module_capacity);
        let mut module_lookup = HashMap::with_capacity(module_capacity);
        for source_module in &unit.modules {
            let key = modules
                .push(HirModule {
                    authoring_namespace_id: source_module.descriptor().authoring_namespace_arc(),
                    imports: TableRange::empty(),
                    source_span: source_module.declaration_span().clone(),
                })
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
            module_lookup.insert(source_module.descriptor().authoring_namespace_arc(), key);
        }

        // 每个模块的导入单独按目标命名空间排序后追加到一个平坦表，TableRange 保留模块
        // 边界并避免每模块 Vec 的额外分配。
        let mut imports = Vec::with_capacity(import_capacity);
        for (module_index, source_module) in unit.modules.iter().enumerate() {
            let module_key = HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
            let start = imports.len();
            let mut canonical_imports: Vec<_> = source_module.import_records().collect();
            canonical_imports.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (target_namespace, source_span) in canonical_imports {
                let target = module_lookup[target_namespace];
                imports.push(HirImport {
                    target,
                    source_span: source_span.clone(),
                });
            }
            modules.get_mut(module_key).imports =
                TableRange::try_from_usize(start, imports.len().saturating_sub(start)).map_err(
                    |overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()),
                )?;
        }

        // 先按 `(canonical module order, stable key)` 为全部声明分配键并建立完整符号表。
        // 这一步必须先于连接解析，才能让前向引用、自环和跨模块引用具有相同语义。
        let mut lane_edges =
            TypedArena::<HirLaneEdgeTag, HirLaneEdge>::with_capacity(lane_edge_capacity);
        let mut symbols = SymbolTable::new(unit.modules.iter().map(|module| {
            module
                .declarations
                .iter()
                .filter(|declaration| matches!(declaration, TypedAstDeclaration::LaneEdge(_)))
                .count()
        }));
        let mut identities = IdentityRegistry::with_capacity(count_to_usize(
            unit.stable_entity_count,
            &unit.limits,
        )?);
        let mut canonical_sources = Vec::with_capacity(lane_edge_capacity);
        for (module_index, source_module) in unit.modules.iter().enumerate() {
            let module_key = HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
            let mut declaration_indices: Vec<usize> =
                (0..source_module.declarations.len()).collect();
            declaration_indices.retain(|index| {
                matches!(
                    source_module.declarations[*index],
                    TypedAstDeclaration::LaneEdge(_)
                )
            });
            declaration_indices.sort_unstable_by_key(|index| {
                &lane_edge_declaration(&source_module.declarations[*index])
                    .expect("filtered declaration must be LaneEdge")
                    .header
                    .source_address
            });
            for declaration_index in declaration_indices {
                let source = lane_edge_declaration(&source_module.declarations[declaration_index])
                    .expect("filtered declaration must be LaneEdge");
                let fields = [
                    IdentityFieldInput::new(
                        FieldTag::AuthoringNamespaceId,
                        source_module
                            .descriptor()
                            .authoring_namespace_id()
                            .as_bytes(),
                    ),
                    IdentityFieldInput::new(
                        FieldTag::LaneEdgeKey,
                        source.header.stable_key.as_bytes(),
                    ),
                ];
                let identity = encode_canonical_identity(
                    EntityKind::LaneEdge,
                    &fields,
                    unit.limits.identity_ascii_bytes_limit(),
                )
                .map_err(|violation| {
                    let mut diagnostic = Diagnostic::invalid_canonical_identity(
                        EntityKind::LaneEdge,
                        &source.header.stable_key,
                        violation,
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    DiagnosticBundle::single(diagnostic)
                })?;
                if let Err(error) = identities.register(&identity, &source.header.span) {
                    let mut diagnostic = match error {
                        IdentityRegistrationError::Duplicate { existing_span } => {
                            Diagnostic::duplicate_canonical_identity(
                                identity.kind(),
                                &source.header.stable_key,
                                identity.stable_id(),
                                source.header.span.clone(),
                                existing_span,
                            )
                        }
                        IdentityRegistrationError::DigestCollision { existing_span } => {
                            Diagnostic::identity_digest_collision(
                                identity.kind(),
                                &source.header.stable_key,
                                identity.stable_id(),
                                source.header.span.clone(),
                                existing_span,
                            )
                        }
                    };
                    diagnostic.set_canonical_module_order(
                        u32::try_from(module_index).unwrap_or(u32::MAX),
                    );
                    return Err(DiagnosticBundle::single(diagnostic));
                }
                let key = lane_edges
                    .push(HirLaneEdge {
                        module: module_key,
                        stable_key: Arc::clone(&source.header.stable_key),
                        source_address: source.header.source_address.clone(),
                        stable_id: LaneEdgeId::from_untyped(identity.stable_id()),
                        length_mm: source
                            .geometry_authority
                            .direct_length()
                            .expect("authoring geometry is compiled before HIR lane construction")
                            .millimetres(),
                        speed_limit_mm_s: source.speed_limit.millimetres_per_second(),
                        successors: TableRange::empty(),
                        source_span: source.header.span.clone(),
                    })
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                    })?;
                symbols.insert(module_key, source.header.source_address.clone(), key);
                canonical_sources.push(CanonicalLaneEdgeSource {
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
                    hir_key: key,
                });
            }
        }
        // 第二遍只解析已经规范化的引用序列。未知目标继续收集到有界诊断集合中；该边的
        // 临时区间不会在失败时泄漏，因为整个 HirUnit 仅在零错误后提交。
        let mut references = Vec::with_capacity(reference_capacity);
        let mut diagnostics =
            DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
        for source_location in canonical_sources {
            let module_index = usize::try_from(source_location.source_module_index)
                .expect("u32 module index must fit usize on supported targets");
            let source_module = &unit.modules[module_index];
            let source = lane_edge_declaration(
                &source_module.declarations[usize::try_from(source_location.declaration_index)
                    .expect("u32 declaration index must fit usize on supported targets")],
            )
            .expect("canonical LaneEdge source must still name a LaneEdge");
            let start = references.len();
            for successor in &source.successors {
                let target_module = module_lookup[successor.module_namespace.as_ref()];
                let Some(target) = symbols.get(target_module, &successor.target_address) else {
                    let mut diagnostic = Diagnostic::unknown_reference_target(
                        EntityKind::LaneEdge,
                        &source.header.stable_key,
                        &successor.module_namespace,
                        successor.declaration_key(),
                        successor.span.clone(),
                        source.header.span.clone(),
                    );
                    diagnostic.set_canonical_module_order(source_location.source_module_index);
                    diagnostics.push(diagnostic);
                    continue;
                };
                references.push(HirLaneEdgeReference {
                    target,
                    source_span: successor.span.clone(),
                });
            }
            lane_edges.get_mut(source_location.hir_key).successors =
                TableRange::try_from_usize(start, references.len().saturating_sub(start)).map_err(
                    |overflow| {
                        arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                    },
                )?;
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        debug_assert_eq!(modules.len(), module_capacity);
        debug_assert_eq!(lane_edges.len(), lane_edge_capacity);
        Ok((
            HirBase {
                modules,
                imports,
                lane_edges,
                lane_edge_references: references,
                module_lookup,
                lane_edge_symbols: symbols,
            },
            identities,
        ))
    }
}

/// 派生并登记规范身份；重复或摘要冲突时返回带规范模块顺序的诊断。
pub(super) fn derive_identity(
    unit: &CompilationUnit,
    identities: &mut IdentityRegistry,
    module_index: usize,
    entity_kind: EntityKind,
    stable_key: &str,
    source_span: &SourceLocation,
    fields: &[IdentityFieldInput<'_>],
) -> Result<StableId128, DiagnosticBundle> {
    let identity = encode_canonical_identity(
        entity_kind,
        fields,
        unit.limits.identity_ascii_bytes_limit(),
    )
    .map_err(|violation| {
        let mut diagnostic = Diagnostic::invalid_canonical_identity(
            entity_kind,
            stable_key,
            violation,
            source_span.clone(),
        );
        diagnostic.set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
        DiagnosticBundle::single(diagnostic)
    })?;
    if let Err(error) = identities.register(&identity, source_span) {
        let mut diagnostic = match error {
            IdentityRegistrationError::Duplicate { existing_span } => {
                Diagnostic::duplicate_canonical_identity(
                    entity_kind,
                    stable_key,
                    identity.stable_id(),
                    source_span.clone(),
                    existing_span,
                )
            }
            IdentityRegistrationError::DigestCollision { existing_span } => {
                Diagnostic::identity_digest_collision(
                    entity_kind,
                    stable_key,
                    identity.stable_id(),
                    source_span.clone(),
                    existing_span,
                )
            }
        };
        diagnostic.set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
        return Err(DiagnosticBundle::single(diagnostic));
    }
    Ok(identity.stable_id())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn register_owner(
    entity_kind: EntityKind,
    target_index: usize,
    target_key: &str,
    owner: HirRoadCorridorKey,
    owner_header: &crate::declaration::DeclarationHeader,
    owners: &mut [Option<(HirRoadCorridorKey, SourceLocation)>],
    corridors: &TypedArena<HirRoadCorridorTag, HirRoadCorridor>,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) {
    if let Some((first_owner, first_span)) = &owners[target_index] {
        let mut diagnostic = Diagnostic::multiple_cross_section_owners(
            entity_kind,
            target_key,
            &corridors.get(*first_owner).stable_key,
            &owner_header.stable_key,
            owner_header.span.clone(),
            first_span.clone(),
        );
        diagnostic.set_canonical_module_order(module_order);
        diagnostics.push(diagnostic);
    } else {
        owners[target_index] = Some((owner, owner_header.span.clone()));
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn resolve_reference<M, K: Copy>(
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    symbols: &SymbolTable<K>,
    reference: &OwnedEntityReference<M>,
    source_kind: EntityKind,
    source_header: &crate::declaration::DeclarationHeader,
    module_order: u32,
    diagnostics: &mut DiagnosticCollector,
) -> Option<K>
where
    M: laneflow_static_contract::EntityKindMarker,
{
    let target_module = module_lookup[reference.module_namespace.as_ref()];
    let Some(target) = symbols.get(target_module, &reference.target_address) else {
        let mut diagnostic = Diagnostic::unknown_owner_qualified_reference_target(
            source_kind,
            &source_header.stable_key,
            &reference.module_namespace,
            reference.target_address.owner_local_keys(),
            reference.declaration_key(),
            reference.span.clone(),
            source_header.span.clone(),
        );
        diagnostic.set_canonical_module_order(module_order);
        diagnostics.push(diagnostic);
        return None;
    };
    Some(target)
}
