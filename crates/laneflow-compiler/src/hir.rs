//! Typed AST 到高层中间表示（HIR）的符号解析阶段。
//!
//! 输入 [`CompilationUnit`] 已闭合模块导入图并冻结依赖优先顺序。本阶段据此建立连续
//! 模块表与分实体符号表，把 `(module namespace, stable key)` 引用解析为阶段私有
//! `u32` 键，并保留来源位置供后续诊断/源映射使用。声明先全部登记、再统一解析引用，
//! 因此前向引用和自环合法；横断面子阶段还在派生子实体身份前证明唯一所有者树。
//!
//! HIR 表顺序是规范顺序：模块沿用编译单元顺序，模块内声明按稳定键排序，导入和连接
//! 也使用已显式规范化的序列。`HashMap` 仅作查找，绝不能通过迭代哈希表决定诊断或
//! 后续布局。所有键、区间和类型均为 crate 私有，不能跨阶段或进入持久制品。

use std::collections::HashMap;
use std::sync::Arc;

use laneflow_static_contract::{
    AuthoringLaneId, EntityKind, FacilityBandId, FieldTag, LaneEdgeId, LaneGroupId, RoadCorridorId,
    RoadSectionId, StableId128,
};

use crate::arena::{ArenaKey, ArenaKeyOverflow, TableRange, TypedArena};
use crate::declaration::{
    LaneEdgeDeclaration, OwnedCorridorElementReference, OwnedEntityReference, SyntheticDeclaration,
};
use crate::diagnostic::DiagnosticCollector;
use crate::identity::{
    IdentityFieldInput, IdentityRegistrationError, IdentityRegistry, RegisteredCanonicalIdentity,
    encode_canonical_identity,
};
use crate::module::SourceDocumentOrdinal;
use crate::{CompilationUnit, CompileLimitDimension, Diagnostic, DiagnosticBundle, SourceSpan};

/// 区分 HIR 模块表键的零尺寸阶段标记。
pub(crate) enum HirModuleTag {}
/// 区分 HIR 车道图边表键的零尺寸阶段标记。
pub(crate) enum HirLaneEdgeTag {}
pub(crate) enum HirRoadCorridorTag {}
pub(crate) enum HirRoadSectionTag {}
pub(crate) enum HirAuthoringLaneTag {}
pub(crate) enum HirLaneGroupTag {}
pub(crate) enum HirFacilityBandTag {}

/// 仅在当前 `HirUnit` 模块表内有效的致密键。
pub(crate) type HirModuleKey = ArenaKey<HirModuleTag>;
/// 仅在当前 `HirUnit` 车道图边表内有效的致密键。
pub(crate) type HirLaneEdgeKey = ArenaKey<HirLaneEdgeTag>;
pub(crate) type HirRoadCorridorKey = ArenaKey<HirRoadCorridorTag>;
pub(crate) type HirRoadSectionKey = ArenaKey<HirRoadSectionTag>;
pub(crate) type HirAuthoringLaneKey = ArenaKey<HirAuthoringLaneTag>;
pub(crate) type HirLaneGroupKey = ArenaKey<HirLaneGroupTag>;
pub(crate) type HirFacilityBandKey = ArenaKey<HirFacilityBandTag>;

/// 已解析为 HIR 模块键的显式导入边。
pub(crate) struct HirImport {
    /// 被导入模块；目标在规范模块顺序中位于当前模块之前。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) target: HirModuleKey,
    /// 原始导入声明位置。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) source_span: SourceSpan,
}

/// HIR 模块记录及其在平坦导入表中的连续区间。
pub(crate) struct HirModule {
    /// 声明身份与跨模块解析使用的稳定命名空间。
    pub(crate) authoring_namespace_id: Arc<str>,
    /// 与机器路径无关的来源文档键。
    pub(crate) source_document_key: Arc<str>,
    /// 编译单元来源文档登记中的显式序号；不能从 `HirModuleKey.raw()` 推断。
    pub(crate) source_document_ordinal: SourceDocumentOrdinal,
    /// 此模块在 `HirUnit::imports` 中的半开区间。
    pub(crate) imports: TableRange<HirImport>,
    /// 模块声明位置。
    pub(crate) source_span: SourceSpan,
}

/// 已解析为 HIR 车道图边键的下游引用。
pub(crate) struct HirLaneEdgeReference {
    /// 当前 `HirUnit::lane_edges` 中的目标键。
    pub(crate) target: HirLaneEdgeKey,
    /// 原始引用位置。
    pub(crate) source_span: SourceSpan,
}

/// 完成模块归属和下游符号解析的车道图边 HIR 记录。
pub(crate) struct HirLaneEdge {
    /// 拥有此声明的 HIR 模块。
    pub(crate) module: HirModuleKey,
    /// 模块内稳定键；不是 HIR 致密下标。
    pub(crate) stable_key: Arc<str>,
    /// 由 `(authoringNamespaceId, laneEdgeKey)` 的完整 Identity v1 前像派生。
    pub(crate) stable_id: LaneEdgeId,
    /// 交通权威长度，单位为米并保留来源 `f64` 精度。
    pub(crate) length_meters: f64,
    /// 基础道路限速，单位为米每秒并保留来源 `f64` 精度。
    pub(crate) speed_limit_meters_per_second: f64,
    /// 此边在 `HirUnit::lane_edge_references` 中的连续下游引用区间。
    pub(crate) successors: TableRange<HirLaneEdgeReference>,
    /// 原始声明位置。
    pub(crate) source_span: SourceSpan,
}

/// 道路走廊有序横断面中的已解析异构成员。
pub(crate) enum HirCorridorElement {
    RoadSection(HirRoadSectionKey),
    FacilityBand(HirFacilityBandKey),
}

/// 已证明参考区段成员性与成员唯一所有权的道路走廊。
pub(crate) struct HirRoadCorridor {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadCorridorId,
    pub(crate) reference_section: HirRoadSectionKey,
    pub(crate) elements: TableRange<HirCorridorElement>,
    pub(crate) source_span: SourceSpan,
}

/// 已闭合到唯一道路走廊父项的道路区段。
pub(crate) struct HirRoadSection {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: RoadSectionId,
    pub(crate) road_corridor: HirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) lanes: TableRange<HirAuthoringLane>,
    pub(crate) source_span: SourceSpan,
}

/// 编制车道覆盖链中的一项已解析车道图边及其来源位置。
pub(crate) struct HirAuthoringLaneEdge {
    pub(crate) target: HirLaneEdgeKey,
    pub(crate) source_span: SourceSpan,
}

/// 已解析父区段、覆盖链和可选车道组的编制车道。
pub(crate) struct HirAuthoringLane {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: AuthoringLaneId,
    pub(crate) road_section: HirRoadSectionKey,
    pub(crate) edge_chain: TableRange<HirAuthoringLaneEdge>,
    pub(crate) lane_group: Option<HirLaneGroupKey>,
    pub(crate) source_span: SourceSpan,
}

/// 车道组成员表中的一条编制车道引用。
#[derive(Clone, Copy)]
pub(crate) struct HirLaneGroupMember {
    pub(crate) lane: HirAuthoringLaneKey,
}

/// 已证明所有成员与父区段一致且非空的车道组。
pub(crate) struct HirLaneGroup {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: LaneGroupId,
    pub(crate) road_section: HirRoadSectionKey,
    pub(crate) members: TableRange<HirLaneGroupMember>,
    pub(crate) source_span: SourceSpan,
}

/// 已闭合到唯一道路走廊父项的非遍历设施带。
pub(crate) struct HirFacilityBand {
    pub(crate) module: HirModuleKey,
    pub(crate) stable_key: Arc<str>,
    pub(crate) stable_id: FacilityBandId,
    pub(crate) road_corridor: HirRoadCorridorKey,
    pub(crate) kind_id: Arc<str>,
    pub(crate) source_span: SourceSpan,
}

/// HIR 阶段成功后一次性冻结的连续只读表集合。
///
/// 构造完成时所有引用均已解析，所有 `TableRange` 都落在对应平坦表内。字段中的键只对
/// 本实例有效。`controlled_live_bytes` 仅统计成功返回后由 HIR 自身持有的阶段字节；
/// 资源预检使用的峰值还包含输入、查找表和暂存区。
pub(crate) struct HirUnit {
    pub(crate) modules: Box<[HirModule]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) imports: Box<[HirImport]>,
    pub(crate) lane_edges: Box<[HirLaneEdge]>,
    pub(crate) lane_edge_references: Box<[HirLaneEdgeReference]>,
    pub(crate) road_corridors: Box<[HirRoadCorridor]>,
    pub(crate) corridor_elements: Box<[HirCorridorElement]>,
    pub(crate) road_sections: Box<[HirRoadSection]>,
    pub(crate) authoring_lanes: Box<[HirAuthoringLane]>,
    pub(crate) authoring_lane_edges: Box<[HirAuthoringLaneEdge]>,
    pub(crate) lane_groups: Box<[HirLaneGroup]>,
    pub(crate) lane_group_members: Box<[HirLaneGroupMember]>,
    pub(crate) facility_bands: Box<[HirFacilityBand]>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) hir_record_count: u64,
    pub(crate) controlled_live_bytes: u64,
}

/// 按 HIR 模块隔离的有类型符号查找索引；不提供规范遍历能力。
struct SymbolTable<K> {
    by_module: Vec<HashMap<Arc<str>, K>>,
}

impl<K: Copy> SymbolTable<K> {
    fn new(module_declaration_counts: impl IntoIterator<Item = usize>) -> Self {
        Self {
            by_module: module_declaration_counts
                .into_iter()
                .map(HashMap::with_capacity)
                .collect(),
        }
    }

    fn insert(&mut self, module: HirModuleKey, stable_key: Arc<str>, key: K) {
        let previous = self.by_module[module.index()].insert(stable_key, key);
        debug_assert!(
            previous.is_none(),
            "Typed AST rejected duplicate declarations"
        );
    }

    fn get(&self, module: HirModuleKey, stable_key: &str) -> Option<K> {
        self.by_module[module.index()].get(stable_key).copied()
    }
}

#[derive(Clone, Copy)]
/// 把规范 HIR 键映回 Typed AST 物理位置的阶段暂存记录。
///
/// HIR 键不能冒充来源模块/声明下标；显式保存两者可在声明排序后仍准确读取来源记录。
struct CanonicalLaneEdgeSource {
    source_module_index: u32,
    declaration_index: u32,
    hir_key: HirLaneEdgeKey,
}

#[derive(Clone, Copy)]
struct CanonicalDeclarationSource<K> {
    source_module_index: u32,
    declaration_index: u32,
    hir_key: K,
}

#[derive(Clone, Copy)]
struct CanonicalAuthoringLaneSource {
    source_module_index: u32,
    declaration_index: u32,
    lane_index: u32,
    hir_key: HirAuthoringLaneKey,
}

#[derive(Default)]
struct CrossSectionHir {
    road_corridors: Box<[HirRoadCorridor]>,
    corridor_elements: Box<[HirCorridorElement]>,
    road_sections: Box<[HirRoadSection]>,
    authoring_lanes: Box<[HirAuthoringLane]>,
    authoring_lane_edges: Box<[HirAuthoringLaneEdge]>,
    lane_groups: Box<[HirLaneGroup]>,
    lane_group_members: Box<[HirLaneGroupMember]>,
    facility_bands: Box<[HirFacilityBand]>,
}

#[derive(Default)]
struct CrossSectionCounts {
    road_corridors: u64,
    corridor_elements: u64,
    road_sections: u64,
    authoring_lanes: u64,
    authoring_lane_edges: u64,
    lane_groups: u64,
    facility_bands: u64,
}

impl CrossSectionCounts {
    fn entity_count(&self) -> u64 {
        self.road_corridors
            .saturating_add(self.road_sections)
            .saturating_add(self.authoring_lanes)
            .saturating_add(self.lane_groups)
            .saturating_add(self.facility_bands)
    }
}

/// 建立模块/符号表，解析车道拓扑，并闭合横断面所有者树与稳定身份。
///
/// # Errors
///
/// 当 HIR 记录数、阶段暂存区、编译器控制存续字节或 `u32` 表边界超过所选配置档，
/// 或任一目标稳定键不存在时，返回规范有序诊断。失败不会返回部分 HIR。
pub(crate) fn build_hir(unit: &CompilationUnit) -> Result<HirUnit, DiagnosticBundle> {
    // 在任何与记录数成正比的阶段分配前，同时预检持久表、lookup 预算和阶段最大暂存区。
    // scratch 取互斥工作集的最大值而非总和，live peak 则包含输入与当时存续的全部集合。
    let module_count = u64::try_from(unit.modules.len()).unwrap_or(u64::MAX);
    let lane_edge_count = lane_edge_count(unit);
    let lane_edge_reference_count = lane_edge_reference_count(unit);
    let cross_section_counts = cross_section_counts(unit);
    let cross_lookup_module_count = if cross_section_counts.entity_count() == 0 {
        0
    } else {
        module_count
    };
    let hir_record_count = module_count
        .saturating_add(unit.import_edge_count)
        .saturating_add(unit.symbol_count)
        .saturating_add(unit.identity_field_occurrence_count)
        .saturating_add(unit.reference_count)
        .saturating_add(unit.relation_occurrence_count);
    let canonical_source_scratch = requested_bytes::<CanonicalLaneEdgeSource>(lane_edge_count)
        .saturating_add(requested_bytes::<usize>(unit.declaration_count));
    let cross_section_scratch = if cross_section_counts.entity_count() == 0 {
        0
    } else {
        requested_bytes::<CanonicalDeclarationSource<HirRoadCorridorKey>>(
            cross_section_counts.road_corridors,
        )
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirRoadSectionKey>,
        >(cross_section_counts.road_sections))
        .saturating_add(requested_bytes::<CanonicalAuthoringLaneSource>(
            cross_section_counts.authoring_lanes,
        ))
        .saturating_add(
            requested_bytes::<CanonicalDeclarationSource<HirLaneGroupKey>>(
                cross_section_counts.lane_groups,
            ),
        )
        .saturating_add(requested_bytes::<
            CanonicalDeclarationSource<HirFacilityBandKey>,
        >(cross_section_counts.facility_bands))
        .saturating_add(requested_bytes::<Option<(HirRoadCorridorKey, SourceSpan)>>(
            cross_section_counts
                .road_sections
                .saturating_add(cross_section_counts.facility_bands),
        ))
        .saturating_add(requested_bytes::<Option<HirAuthoringLaneKey>>(
            lane_edge_count,
        ))
        .saturating_add(requested_bytes::<usize>(
            cross_section_counts.lane_groups.saturating_mul(2),
        ))
        .saturating_add(requested_bytes::<usize>(unit.declaration_count))
    };
    let import_sort_scratch = requested_bytes::<(&str, &SourceSpan)>(unit.import_edge_count);
    let (canonical_identity_bytes, largest_canonical_identity_bytes) = identity_byte_counts(unit);
    let stage_scratch_bytes = canonical_source_scratch
        .max(cross_section_scratch)
        .max(import_sort_scratch)
        .max(largest_canonical_identity_bytes);
    let hir_persistent_bytes = requested_bytes::<HirModule>(module_count)
        .saturating_add(requested_bytes::<HirImport>(unit.import_edge_count))
        .saturating_add(requested_bytes::<HirLaneEdge>(lane_edge_count))
        .saturating_add(requested_bytes::<HirLaneEdgeReference>(
            lane_edge_reference_count,
        ))
        .saturating_add(requested_bytes::<HirRoadCorridor>(
            cross_section_counts.road_corridors,
        ))
        .saturating_add(requested_bytes::<HirCorridorElement>(
            cross_section_counts.corridor_elements,
        ))
        .saturating_add(requested_bytes::<HirRoadSection>(
            cross_section_counts.road_sections,
        ))
        .saturating_add(requested_bytes::<HirAuthoringLane>(
            cross_section_counts.authoring_lanes,
        ))
        .saturating_add(requested_bytes::<HirAuthoringLaneEdge>(
            cross_section_counts.authoring_lane_edges,
        ))
        .saturating_add(requested_bytes::<HirLaneGroup>(
            cross_section_counts.lane_groups,
        ))
        .saturating_add(requested_bytes::<HirLaneGroupMember>(
            cross_section_counts.authoring_lanes,
        ))
        .saturating_add(requested_bytes::<HirFacilityBand>(
            cross_section_counts.facility_bands,
        ));
    let hir_lookup_bytes = requested_hash_table_bytes::<Arc<str>, HirModuleKey>(module_count)
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirLaneEdgeKey>>(
            module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirRoadSectionKey>>(
            cross_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirLaneGroupKey>>(
            cross_lookup_module_count,
        ))
        .saturating_add(requested_bytes::<HashMap<Arc<str>, HirFacilityBandKey>>(
            cross_lookup_module_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirLaneEdgeKey>(
            lane_edge_count,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirRoadSectionKey>(
            cross_section_counts.road_sections,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirLaneGroupKey>(
            cross_section_counts.lane_groups,
        ))
        .saturating_add(requested_hash_table_bytes::<Arc<str>, HirFacilityBandKey>(
            cross_section_counts.facility_bands,
        ))
        .saturating_add(requested_hash_table_bytes::<
            StableId128,
            RegisteredCanonicalIdentity,
        >(unit.declaration_count))
        .saturating_add(canonical_identity_bytes);
    let controlled_live_bytes = unit
        .controlled_live_bytes
        .saturating_add(hir_persistent_bytes)
        .saturating_add(hir_lookup_bytes)
        .saturating_add(stage_scratch_bytes);

    let primary_span = unit
        .modules
        .first()
        .map(|module| module.descriptor().declaration_span().clone());
    let stable_key = unit
        .modules
        .first()
        .map(|module| module.descriptor().authoring_namespace_id().into());
    let mut limit_diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    for (dimension, observed) in [
        (CompileLimitDimension::HirRecordCount, hir_record_count),
        (
            CompileLimitDimension::StageScratchBytes,
            stage_scratch_bytes,
        ),
        (
            CompileLimitDimension::CompilerControlledLiveBytes,
            controlled_live_bytes,
        ),
    ] {
        if observed > unit.limits.value(dimension) {
            limit_diagnostics.push(Diagnostic::compile_limit_exceeded_at(
                dimension,
                unit.limits.value(dimension),
                observed,
                primary_span.clone(),
                stable_key.clone(),
            ));
        }
    }
    if !limit_diagnostics.is_empty() {
        return Err(limit_diagnostics.finish());
    }

    let module_capacity = unit.modules.len();
    let import_capacity = count_to_usize(unit.import_edge_count, &unit.limits)?;
    let lane_edge_capacity = count_to_usize(lane_edge_count, &unit.limits)?;
    let reference_capacity = count_to_usize(lane_edge_reference_count, &unit.limits)?;
    // 第一阶段冻结模块键。CompilationUnit 已按依赖优先排序，因此 raw key 顺序可直接
    // 作为后续规范模块轴；module_lookup 只用于解析，不参与任何输出遍历。
    let mut modules = TypedArena::<HirModuleTag, HirModule>::with_capacity(module_capacity);
    let mut module_lookup = HashMap::with_capacity(module_capacity);
    for (source_document_index, source_module) in unit.modules.iter().enumerate() {
        let source_document_ordinal =
            SourceDocumentOrdinal::from_raw(u32::try_from(source_document_index).map_err(
                |_| arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone()),
            )?);
        let key = modules
            .push(HirModule {
                authoring_namespace_id: source_module.descriptor().authoring_namespace_arc(),
                source_document_key: source_module.descriptor().source_document_key_arc(),
                source_document_ordinal,
                imports: TableRange::empty(),
                source_span: source_module.descriptor().declaration_span().clone(),
            })
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
        module_lookup.insert(source_module.descriptor().authoring_namespace_arc(), key);
    }

    // 每个模块的导入单独按目标命名空间排序后追加到一个平坦表，TableRange 保留模块
    // 边界并避免每模块 Vec 的额外分配。
    let mut imports = Vec::with_capacity(import_capacity);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key =
            HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
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
            TableRange::try_from_usize(start, imports.len().saturating_sub(start))
                .map_err(|overflow| arena_overflow(overflow, &unit.limits, primary_span.clone()))?;
    }

    // 先按 `(canonical module order, stable key)` 为全部声明分配键并建立完整符号表。
    // 这一步必须先于连接解析，才能让前向引用、自环和跨模块引用具有相同语义。
    let mut lane_edges =
        TypedArena::<HirLaneEdgeTag, HirLaneEdge>::with_capacity(lane_edge_capacity);
    let mut symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, SyntheticDeclaration::LaneEdge(_)))
            .count()
    }));
    let mut identities =
        IdentityRegistry::with_capacity(count_to_usize(unit.declaration_count, &unit.limits)?);
    let mut canonical_sources = Vec::with_capacity(lane_edge_capacity);
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key =
            HirModuleKey::from_raw(u32::try_from(module_index).map_err(|_| {
                arena_overflow(ArenaKeyOverflow, &unit.limits, primary_span.clone())
            })?);
        let mut declaration_indices: Vec<usize> = (0..source_module.declarations.len()).collect();
        declaration_indices.retain(|index| {
            matches!(
                source_module.declarations[*index],
                SyntheticDeclaration::LaneEdge(_)
            )
        });
        declaration_indices.sort_unstable_by(|left, right| {
            lane_edge_declaration(&source_module.declarations[*left])
                .expect("filtered declaration must be LaneEdge")
                .header
                .stable_key
                .cmp(
                    &lane_edge_declaration(&source_module.declarations[*right])
                        .expect("filtered declaration must be LaneEdge")
                        .header
                        .stable_key,
                )
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
                IdentityFieldInput::new(FieldTag::LaneEdgeKey, source.header.stable_key.as_bytes()),
            ];
            let identity = encode_canonical_identity(
                EntityKind::LaneEdge,
                &fields,
                unit.limits.value(CompileLimitDimension::SingleStringBytes),
            )
            .map_err(|violation| {
                let mut diagnostic = Diagnostic::invalid_canonical_identity(
                    EntityKind::LaneEdge,
                    &source.header.stable_key,
                    violation,
                    source.header.span.clone(),
                );
                diagnostic
                    .set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
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
                diagnostic
                    .set_canonical_module_order(u32::try_from(module_index).unwrap_or(u32::MAX));
                return Err(DiagnosticBundle::single(diagnostic));
            }
            let key = lane_edges
                .push(HirLaneEdge {
                    module: module_key,
                    stable_key: Arc::clone(&source.header.stable_key),
                    stable_id: LaneEdgeId::from_untyped(identity.stable_id()),
                    length_meters: source.length.value(),
                    speed_limit_meters_per_second: source.speed_limit.value(),
                    successors: TableRange::empty(),
                    source_span: source.header.span.clone(),
                })
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
            symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
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
            let Some(target) = symbols.get(target_module, &successor.declaration_key) else {
                let mut diagnostic = Diagnostic::unknown_reference_target(
                    EntityKind::LaneEdge,
                    &source.header.stable_key,
                    &successor.module_namespace,
                    &successor.declaration_key,
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
                |overflow| arena_overflow(overflow, &unit.limits, Some(source.header.span.clone())),
            )?;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let cross_section = build_cross_section_hir(
        unit,
        &module_lookup,
        &lane_edges,
        &references,
        &symbols,
        &mut identities,
    )?;
    // 完整规范前像只服务本阶段的重复/碰撞判断。此后各表仅保留 16 字节有类型 ID，
    // 避免在 HIR 与 MIR 中复制可由稳定键和父项重建的 identity envelope。
    drop(identities);

    debug_assert_eq!(modules.len(), module_capacity);
    debug_assert_eq!(lane_edges.len(), lane_edge_capacity);
    Ok(HirUnit {
        modules: modules.into_boxed_slice(),
        imports: imports.into_boxed_slice(),
        lane_edges: lane_edges.into_boxed_slice(),
        lane_edge_references: references.into_boxed_slice(),
        road_corridors: cross_section.road_corridors,
        corridor_elements: cross_section.corridor_elements,
        road_sections: cross_section.road_sections,
        authoring_lanes: cross_section.authoring_lanes,
        authoring_lane_edges: cross_section.authoring_lane_edges,
        lane_groups: cross_section.lane_groups,
        lane_group_members: cross_section.lane_group_members,
        facility_bands: cross_section.facility_bands,
        hir_record_count,
        controlled_live_bytes: hir_persistent_bytes,
    })
}

#[allow(clippy::too_many_lines)]
fn build_cross_section_hir(
    unit: &CompilationUnit,
    module_lookup: &HashMap<Arc<str>, HirModuleKey>,
    lane_edges: &TypedArena<HirLaneEdgeTag, HirLaneEdge>,
    lane_edge_references: &[HirLaneEdgeReference],
    lane_edge_symbols: &SymbolTable<HirLaneEdgeKey>,
    identities: &mut IdentityRegistry,
) -> Result<CrossSectionHir, DiagnosticBundle> {
    let counts = cross_section_counts(unit);
    if counts.entity_count() == 0 {
        return Ok(CrossSectionHir::default());
    }
    // 只为会被引用解析访问的实体建立符号表，并按实体类别精确预留容量。RoadCorridor
    // 与 AuthoringLane 在本切片中没有按键引用消费者；为它们建立查找表只会增加峰值内存。
    let mut section_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, SyntheticDeclaration::RoadSection(_)))
            .count()
    }));
    let mut group_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, SyntheticDeclaration::LaneGroup(_)))
            .count()
    }));
    let mut band_symbols = SymbolTable::new(unit.modules.iter().map(|module| {
        module
            .declarations
            .iter()
            .filter(|declaration| matches!(declaration, SyntheticDeclaration::FacilityBand(_)))
            .count()
    }));

    let mut corridors = TypedArena::<HirRoadCorridorTag, HirRoadCorridor>::with_capacity(
        count_to_usize(counts.road_corridors, &unit.limits)?,
    );
    let mut sections = TypedArena::<HirRoadSectionTag, HirRoadSection>::with_capacity(
        count_to_usize(counts.road_sections, &unit.limits)?,
    );
    let mut lanes = TypedArena::<HirAuthoringLaneTag, HirAuthoringLane>::with_capacity(
        count_to_usize(counts.authoring_lanes, &unit.limits)?,
    );
    let mut groups = TypedArena::<HirLaneGroupTag, HirLaneGroup>::with_capacity(count_to_usize(
        counts.lane_groups,
        &unit.limits,
    )?);
    let mut bands = TypedArena::<HirFacilityBandTag, HirFacilityBand>::with_capacity(
        count_to_usize(counts.facility_bands, &unit.limits)?,
    );
    let mut corridor_sources = Vec::with_capacity(corridors_capacity(&counts, &unit.limits)?);
    let mut section_sources = Vec::with_capacity(sections_capacity(&counts, &unit.limits)?);
    let mut lane_sources = Vec::with_capacity(lanes_capacity(&counts, &unit.limits)?);
    let mut group_sources = Vec::with_capacity(groups_capacity(&counts, &unit.limits)?);
    let mut band_sources = Vec::with_capacity(bands_capacity(&counts, &unit.limits)?);

    // 首遍只登记符号与不依赖父项的 RoadCorridor identity。其余实体先保留零值占位，
    // 但在所有者/引用错误存在时不会逃逸出本函数；父项闭合后才写入真实 ID。
    for (module_index, source_module) in unit.modules.iter().enumerate() {
        let module_key = HirModuleKey::from_raw(
            u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        let mut declaration_indices: Vec<_> = source_module
            .declarations
            .iter()
            .enumerate()
            .filter_map(|(index, declaration)| {
                (!matches!(declaration, SyntheticDeclaration::LaneEdge(_))).then_some(index)
            })
            .collect();
        declaration_indices.sort_unstable_by(|left, right| {
            let left = declaration_header(&source_module.declarations[*left]);
            let right = declaration_header(&source_module.declarations[*right]);
            (left.entity_kind.code(), left.stable_key.as_bytes())
                .cmp(&(right.entity_kind.code(), right.stable_key.as_bytes()))
        });
        for source_declaration_index in declaration_indices {
            let source_module_index = u32::try_from(module_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            let declaration_index = u32::try_from(source_declaration_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?;
            match &source_module.declarations[source_declaration_index] {
                SyntheticDeclaration::LaneEdge(_) => {
                    unreachable!("cross-section source filter admitted LaneEdge")
                }
                SyntheticDeclaration::RoadCorridor(source) => {
                    let fields = [
                        IdentityFieldInput::new(
                            FieldTag::AuthoringNamespaceId,
                            source_module
                                .descriptor()
                                .authoring_namespace_id()
                                .as_bytes(),
                        ),
                        IdentityFieldInput::new(
                            FieldTag::CorridorKey,
                            source.header.stable_key.as_bytes(),
                        ),
                    ];
                    let stable_id = RoadCorridorId::from_untyped(derive_identity(
                        unit,
                        identities,
                        module_index,
                        EntityKind::RoadCorridor,
                        &source.header.stable_key,
                        &source.header.span,
                        &fields,
                    )?);
                    let key = corridors
                        .push(HirRoadCorridor {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id,
                            reference_section: HirRoadSectionKey::from_raw(0),
                            elements: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    corridor_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                SyntheticDeclaration::RoadSection(source) => {
                    let lane_start = lanes.len();
                    let section_key = sections
                        .push(HirRoadSection {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: RoadSectionId::from_untyped(StableId128::ZERO),
                            road_corridor: HirRoadCorridorKey::from_raw(0),
                            kind_id: Arc::clone(&source.kind_id),
                            lanes: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    section_symbols.insert(
                        module_key,
                        Arc::clone(&source.header.stable_key),
                        section_key,
                    );
                    section_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: section_key,
                    });
                    for (lane_index, lane) in source.lanes.iter().enumerate() {
                        let lane_key = lanes
                            .push(HirAuthoringLane {
                                module: module_key,
                                stable_key: Arc::clone(&lane.header.stable_key),
                                stable_id: AuthoringLaneId::from_untyped(StableId128::ZERO),
                                road_section: section_key,
                                edge_chain: TableRange::empty(),
                                lane_group: None,
                                source_span: lane.header.span.clone(),
                            })
                            .map_err(|overflow| {
                                arena_overflow(
                                    overflow,
                                    &unit.limits,
                                    Some(lane.header.span.clone()),
                                )
                            })?;
                        lane_sources.push(CanonicalAuthoringLaneSource {
                            source_module_index,
                            declaration_index,
                            lane_index: u32::try_from(lane_index).map_err(|_| {
                                arena_overflow(
                                    ArenaKeyOverflow,
                                    &unit.limits,
                                    Some(lane.header.span.clone()),
                                )
                            })?,
                            hir_key: lane_key,
                        });
                    }
                    sections.get_mut(section_key).lanes = TableRange::try_from_usize(
                        lane_start,
                        lanes.len().saturating_sub(lane_start),
                    )
                    .map_err(|overflow| {
                        arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                    })?;
                }
                SyntheticDeclaration::LaneGroup(source) => {
                    let key = groups
                        .push(HirLaneGroup {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: LaneGroupId::from_untyped(StableId128::ZERO),
                            road_section: HirRoadSectionKey::from_raw(0),
                            members: TableRange::empty(),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    group_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    group_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
                SyntheticDeclaration::FacilityBand(source) => {
                    let key = bands
                        .push(HirFacilityBand {
                            module: module_key,
                            stable_key: Arc::clone(&source.header.stable_key),
                            stable_id: FacilityBandId::from_untyped(StableId128::ZERO),
                            road_corridor: HirRoadCorridorKey::from_raw(0),
                            kind_id: Arc::clone(&source.kind_id),
                            source_span: source.header.span.clone(),
                        })
                        .map_err(|overflow| {
                            arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                        })?;
                    band_symbols.insert(module_key, Arc::clone(&source.header.stable_key), key);
                    band_sources.push(CanonicalDeclarationSource {
                        source_module_index,
                        declaration_index,
                        hir_key: key,
                    });
                }
            }
        }
    }

    let mut diagnostics =
        DiagnosticCollector::new(unit.limits.value(CompileLimitDimension::DiagnosticCount));
    let mut corridor_elements =
        Vec::with_capacity(count_to_usize(counts.corridor_elements, &unit.limits)?);
    let mut section_owners: Vec<Option<(HirRoadCorridorKey, SourceSpan)>> =
        vec![None; sections.len()];
    let mut band_owners: Vec<Option<(HirRoadCorridorKey, SourceSpan)>> = vec![None; bands.len()];

    for location in &corridor_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let SyntheticDeclaration::RoadCorridor(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical RoadCorridor source changed kind")
        };
        let reference_section = resolve_reference(
            module_lookup,
            &section_symbols,
            &source.reference_section,
            EntityKind::RoadCorridor,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        );
        let start = corridor_elements.len();
        let mut reference_is_member = false;
        for element in &source.elements {
            match element {
                OwnedCorridorElementReference::RoadSection(reference) => {
                    if let Some(target) = resolve_reference(
                        module_lookup,
                        &section_symbols,
                        reference,
                        EntityKind::RoadCorridor,
                        &source.header,
                        location.source_module_index,
                        &mut diagnostics,
                    ) {
                        reference_is_member |= reference_section == Some(target);
                        register_owner(
                            EntityKind::RoadSection,
                            target.index(),
                            &sections.get(target).stable_key,
                            location.hir_key,
                            &source.header,
                            &mut section_owners,
                            &corridors,
                            location.source_module_index,
                            &mut diagnostics,
                        );
                        corridor_elements.push(HirCorridorElement::RoadSection(target));
                    }
                }
                OwnedCorridorElementReference::FacilityBand(reference) => {
                    if let Some(target) = resolve_reference(
                        module_lookup,
                        &band_symbols,
                        reference,
                        EntityKind::RoadCorridor,
                        &source.header,
                        location.source_module_index,
                        &mut diagnostics,
                    ) {
                        register_owner(
                            EntityKind::FacilityBand,
                            target.index(),
                            &bands.get(target).stable_key,
                            location.hir_key,
                            &source.header,
                            &mut band_owners,
                            &corridors,
                            location.source_module_index,
                            &mut diagnostics,
                        );
                        corridor_elements.push(HirCorridorElement::FacilityBand(target));
                    }
                }
            }
        }
        if let Some(reference_section) = reference_section {
            corridors.get_mut(location.hir_key).reference_section = reference_section;
            if !reference_is_member {
                let mut diagnostic = Diagnostic::invalid_corridor_reference_section(
                    &source.header.stable_key,
                    &source.reference_section.module_namespace,
                    &source.reference_section.declaration_key,
                    source.reference_section.span.clone(),
                    source.header.span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            }
        }
        corridors.get_mut(location.hir_key).elements =
            TableRange::try_from_usize(start, corridor_elements.len().saturating_sub(start))
                .map_err(|overflow| {
                    arena_overflow(overflow, &unit.limits, Some(source.header.span.clone()))
                })?;
    }

    for (key, section) in sections.iter() {
        if section_owners[key.index()].is_none() {
            let mut diagnostic = Diagnostic::missing_cross_section_owner(
                EntityKind::RoadSection,
                &section.stable_key,
                section.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(section.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    for (key, band) in bands.iter() {
        if band_owners[key.index()].is_none() {
            let mut diagnostic = Diagnostic::missing_cross_section_owner(
                EntityKind::FacilityBand,
                &band.stable_key,
                band.source_span.clone(),
            );
            diagnostic.set_canonical_module_order(band.module.raw());
            diagnostics.push(diagnostic);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 父走廊已唯一闭合，此时才派生 RoadSection / FacilityBand identity。
    for location in &section_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let SyntheticDeclaration::RoadSection(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical RoadSection source changed kind")
        };
        let owner = section_owners[location.hir_key.index()]
            .as_ref()
            .expect("owner diagnostics already rejected missing sections")
            .0;
        let owner_id = corridors.get(owner).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::SectionKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadCorridorStableId,
                owner_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = RoadSectionId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::RoadSection,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let section = sections.get_mut(location.hir_key);
        section.road_corridor = owner;
        section.stable_id = stable_id;
    }
    for location in &band_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let SyntheticDeclaration::FacilityBand(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical FacilityBand source changed kind")
        };
        let owner = band_owners[location.hir_key.index()]
            .as_ref()
            .expect("owner diagnostics already rejected missing bands")
            .0;
        let owner_id = corridors.get(owner).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::FacilityBandKey,
                source.header.stable_key.as_bytes(),
            ),
            IdentityFieldInput::new(
                FieldTag::RoadCorridorStableId,
                owner_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = FacilityBandId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::FacilityBand,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let band = bands.get_mut(location.hir_key);
        band.road_corridor = owner;
        band.stable_id = stable_id;
    }

    // LaneGroup 的父区段是其 identity 输入，必须先解析再处理引用它的编制车道。
    for location in &group_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let SyntheticDeclaration::LaneGroup(source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical LaneGroup source changed kind")
        };
        let Some(parent) = resolve_reference(
            module_lookup,
            &section_symbols,
            &source.road_section,
            EntityKind::LaneGroup,
            &source.header,
            location.source_module_index,
            &mut diagnostics,
        ) else {
            continue;
        };
        let parent_id = sections.get(parent).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::LaneGroupKey, source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadSectionStableId,
                parent_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = LaneGroupId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::LaneGroup,
            &source.header.stable_key,
            &source.header.span,
            &fields,
        )?);
        let group = groups.get_mut(location.hir_key);
        group.road_section = parent;
        group.stable_id = stable_id;
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    let mut lane_edges_flat =
        Vec::with_capacity(count_to_usize(counts.authoring_lane_edges, &unit.limits)?);
    let mut edge_owners: Vec<Option<HirAuthoringLaneKey>> = vec![None; lane_edges.len()];
    let mut group_member_counts = vec![0_usize; groups.len()];
    for location in &lane_sources {
        let source_module = &unit.modules[location.source_module_index as usize];
        let SyntheticDeclaration::RoadSection(section_source) =
            &source_module.declarations[location.declaration_index as usize]
        else {
            unreachable!("canonical AuthoringLane source parent changed kind")
        };
        let lane_source = &section_source.lanes[location.lane_index as usize];
        let parent = lanes.get(location.hir_key).road_section;
        let parent_id = sections.get(parent).stable_id;
        let fields = [
            IdentityFieldInput::new(
                FieldTag::AuthoringNamespaceId,
                source_module
                    .descriptor()
                    .authoring_namespace_id()
                    .as_bytes(),
            ),
            IdentityFieldInput::new(FieldTag::LaneKey, lane_source.header.stable_key.as_bytes()),
            IdentityFieldInput::new(
                FieldTag::RoadSectionStableId,
                parent_id.as_untyped().as_bytes(),
            ),
        ];
        let stable_id = AuthoringLaneId::from_untyped(derive_identity(
            unit,
            identities,
            location.source_module_index as usize,
            EntityKind::AuthoringLane,
            &lane_source.header.stable_key,
            &lane_source.header.span,
            &fields,
        )?);
        let start = lane_edges_flat.len();
        let mut predecessor = None;
        for reference in &lane_source.edge_chain {
            let Some(target) = resolve_reference(
                module_lookup,
                lane_edge_symbols,
                reference,
                EntityKind::AuthoringLane,
                &lane_source.header,
                location.source_module_index,
                &mut diagnostics,
            ) else {
                // 未知引用保留自身诊断，但不能把其两侧原本不相邻的边拼接后再检查连通性。
                predecessor = None;
                continue;
            };
            if let Some(first_owner) = edge_owners[target.index()] {
                let mut diagnostic = Diagnostic::multiple_authoring_lane_owners(
                    &lane_edges.get(target).stable_key,
                    &lanes.get(first_owner).stable_key,
                    &lane_source.header.stable_key,
                    reference.span.clone(),
                    lanes.get(first_owner).source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                edge_owners[target.index()] = Some(location.hir_key);
            }
            if let Some((predecessor_key, predecessor_span)) = predecessor {
                let predecessor_record = lane_edges.get(predecessor_key);
                let connected = lane_edge_references
                    [predecessor_record.successors.as_usize_range()]
                .iter()
                .any(|candidate| candidate.target == target);
                if !connected {
                    let mut diagnostic = Diagnostic::disconnected_authoring_lane_edge_chain(
                        &lane_source.header.stable_key,
                        &predecessor_record.stable_key,
                        &lane_edges.get(target).stable_key,
                        reference.span.clone(),
                        predecessor_span,
                    );
                    diagnostic.set_canonical_module_order(location.source_module_index);
                    diagnostics.push(diagnostic);
                }
            }
            predecessor = Some((target, reference.span.clone()));
            lane_edges_flat.push(HirAuthoringLaneEdge {
                target,
                source_span: reference.span.clone(),
            });
        }

        let lane_group = lane_source.lane_group.as_ref().and_then(|reference| {
            resolve_reference(
                module_lookup,
                &group_symbols,
                reference,
                EntityKind::AuthoringLane,
                &lane_source.header,
                location.source_module_index,
                &mut diagnostics,
            )
        });
        if let Some(group_key) = lane_group {
            let group = groups.get(group_key);
            if group.road_section != parent {
                let mut diagnostic = Diagnostic::lane_group_parent_mismatch(
                    &lane_source.header.stable_key,
                    &group.stable_key,
                    &sections.get(parent).stable_key,
                    &sections.get(group.road_section).stable_key,
                    lane_source
                        .lane_group
                        .as_ref()
                        .expect("resolved lane group has source reference")
                        .span
                        .clone(),
                    group.source_span.clone(),
                );
                diagnostic.set_canonical_module_order(location.source_module_index);
                diagnostics.push(diagnostic);
            } else {
                group_member_counts[group_key.index()] =
                    group_member_counts[group_key.index()].saturating_add(1);
            }
        }
        let lane = lanes.get_mut(location.hir_key);
        lane.stable_id = stable_id;
        lane.edge_chain =
            TableRange::try_from_usize(start, lane_edges_flat.len().saturating_sub(start))
                .map_err(|overflow| {
                    arena_overflow(
                        overflow,
                        &unit.limits,
                        Some(lane_source.header.span.clone()),
                    )
                })?;
        lane.lane_group = lane_group;
    }

    for (group_key, group) in groups.iter() {
        if group_member_counts[group_key.index()] == 0 {
            diagnostics.push(Diagnostic::empty_lane_group(
                &group.stable_key,
                group.source_span.clone(),
            ));
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics.finish());
    }

    // 先按 group key 计算连续范围，再按 lane key 递增顺序填充。这样维持与原车道遍历
    // 一致的成员顺序，同时避免为每个 LaneGroup 单独分配一个临时 Vec。
    let mut next_group_member = Vec::with_capacity(groups.len());
    let mut member_count = 0_usize;
    for (group_index, count) in group_member_counts.iter().copied().enumerate() {
        next_group_member.push(member_count);
        let group_key = HirLaneGroupKey::from_raw(
            u32::try_from(group_index)
                .map_err(|_| arena_overflow(ArenaKeyOverflow, &unit.limits, None))?,
        );
        groups.get_mut(group_key).members = TableRange::try_from_usize(member_count, count)
            .map_err(|overflow| arena_overflow(overflow, &unit.limits, None))?;
        member_count = member_count.saturating_add(count);
    }
    let mut lane_group_members = if member_count == 0 {
        Vec::new()
    } else {
        let first_member = lanes
            .iter()
            .find_map(|(key, lane)| lane.lane_group.map(|_| key))
            .expect("positive validated group member count must name a lane");
        vec![HirLaneGroupMember { lane: first_member }; member_count]
    };
    for (lane_key, lane) in lanes.iter() {
        let Some(group_key) = lane.lane_group else {
            continue;
        };
        let destination = &mut next_group_member[group_key.index()];
        lane_group_members[*destination] = HirLaneGroupMember { lane: lane_key };
        *destination += 1;
    }
    debug_assert!(groups.iter().all(|(key, group)| {
        next_group_member[key.index()] == group.members.as_usize_range().end
    }));

    Ok(CrossSectionHir {
        road_corridors: corridors.into_boxed_slice(),
        corridor_elements: corridor_elements.into_boxed_slice(),
        road_sections: sections.into_boxed_slice(),
        authoring_lanes: lanes.into_boxed_slice(),
        authoring_lane_edges: lane_edges_flat.into_boxed_slice(),
        lane_groups: groups.into_boxed_slice(),
        lane_group_members: lane_group_members.into_boxed_slice(),
        facility_bands: bands.into_boxed_slice(),
    })
}

fn derive_identity(
    unit: &CompilationUnit,
    identities: &mut IdentityRegistry,
    module_index: usize,
    entity_kind: EntityKind,
    stable_key: &str,
    source_span: &SourceSpan,
    fields: &[IdentityFieldInput<'_>],
) -> Result<StableId128, DiagnosticBundle> {
    let identity = encode_canonical_identity(
        entity_kind,
        fields,
        unit.limits.value(CompileLimitDimension::SingleStringBytes),
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
fn register_owner(
    entity_kind: EntityKind,
    target_index: usize,
    target_key: &str,
    owner: HirRoadCorridorKey,
    owner_header: &crate::declaration::DeclarationHeader,
    owners: &mut [Option<(HirRoadCorridorKey, SourceSpan)>],
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
fn resolve_reference<M, K: Copy>(
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
    let Some(target) = symbols.get(target_module, &reference.declaration_key) else {
        let mut diagnostic = Diagnostic::unknown_reference_target(
            source_kind,
            &source_header.stable_key,
            &reference.module_namespace,
            &reference.declaration_key,
            reference.span.clone(),
            source_header.span.clone(),
        );
        diagnostic.set_canonical_module_order(module_order);
        diagnostics.push(diagnostic);
        return None;
    };
    Some(target)
}

fn lane_edge_declaration(declaration: &SyntheticDeclaration) -> Option<&LaneEdgeDeclaration> {
    match declaration {
        SyntheticDeclaration::LaneEdge(declaration) => Some(declaration),
        _ => None,
    }
}

fn declaration_header(
    declaration: &SyntheticDeclaration,
) -> &crate::declaration::DeclarationHeader {
    match declaration {
        SyntheticDeclaration::LaneEdge(declaration) => &declaration.header,
        SyntheticDeclaration::RoadCorridor(declaration) => &declaration.header,
        SyntheticDeclaration::RoadSection(declaration) => &declaration.header,
        SyntheticDeclaration::LaneGroup(declaration) => &declaration.header,
        SyntheticDeclaration::FacilityBand(declaration) => &declaration.header,
    }
}

fn lane_edge_count(unit: &CompilationUnit) -> u64 {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .filter(|declaration| matches!(declaration, SyntheticDeclaration::LaneEdge(_)))
        .count()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn lane_edge_reference_count(unit: &CompilationUnit) -> u64 {
    unit.modules
        .iter()
        .flat_map(|module| module.declarations.iter())
        .filter_map(lane_edge_declaration)
        .fold(0_u64, |total, declaration| {
            total.saturating_add(u64::try_from(declaration.successors.len()).unwrap_or(u64::MAX))
        })
}

fn cross_section_counts(unit: &CompilationUnit) -> CrossSectionCounts {
    let mut counts = CrossSectionCounts::default();
    for declaration in unit
        .modules
        .iter()
        .flat_map(|module| module.declarations.iter())
    {
        match declaration {
            SyntheticDeclaration::LaneEdge(_) => {}
            SyntheticDeclaration::RoadCorridor(corridor) => {
                counts.road_corridors = counts.road_corridors.saturating_add(1);
                counts.corridor_elements = counts
                    .corridor_elements
                    .saturating_add(u64::try_from(corridor.elements.len()).unwrap_or(u64::MAX));
            }
            SyntheticDeclaration::RoadSection(section) => {
                counts.road_sections = counts.road_sections.saturating_add(1);
                counts.authoring_lanes = counts
                    .authoring_lanes
                    .saturating_add(u64::try_from(section.lanes.len()).unwrap_or(u64::MAX));
                counts.authoring_lane_edges =
                    counts
                        .authoring_lane_edges
                        .saturating_add(section.lanes.iter().fold(0_u64, |total, lane| {
                            total.saturating_add(
                                u64::try_from(lane.edge_chain.len()).unwrap_or(u64::MAX),
                            )
                        }));
            }
            SyntheticDeclaration::LaneGroup(_) => {
                counts.lane_groups = counts.lane_groups.saturating_add(1);
            }
            SyntheticDeclaration::FacilityBand(_) => {
                counts.facility_bands = counts.facility_bands.saturating_add(1);
            }
        }
    }
    counts
}

fn corridors_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.road_corridors, limits)
}

fn sections_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.road_sections, limits)
}

fn lanes_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.authoring_lanes, limits)
}

fn groups_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.lane_groups, limits)
}

fn bands_capacity(
    counts: &CrossSectionCounts,
    limits: &crate::CompileLimits,
) -> Result<usize, DiagnosticBundle> {
    count_to_usize(counts.facility_bands, limits)
}

fn identity_byte_counts(unit: &CompilationUnit) -> (u64, u64) {
    let mut total = 0_u64;
    let mut largest = 0_u64;
    for module in &unit.modules {
        let namespace_bytes =
            u64::try_from(module.descriptor().authoring_namespace_id().len()).unwrap_or(u64::MAX);
        for source_declaration in &module.declarations {
            let header = declaration_header(source_declaration);
            let parent_bytes = match source_declaration {
                SyntheticDeclaration::LaneEdge(_) | SyntheticDeclaration::RoadCorridor(_) => 0,
                SyntheticDeclaration::RoadSection(_)
                | SyntheticDeclaration::LaneGroup(_)
                | SyntheticDeclaration::FacilityBand(_) => 22,
            };
            let bytes = 22_u64
                .saturating_add(namespace_bytes)
                .saturating_add(u64::try_from(header.stable_key.len()).unwrap_or(u64::MAX))
                .saturating_add(parent_bytes);
            total = total.saturating_add(bytes);
            largest = largest.max(bytes);
            if let SyntheticDeclaration::RoadSection(section) = source_declaration {
                for lane in &section.lanes {
                    let lane_bytes = 44_u64.saturating_add(namespace_bytes).saturating_add(
                        u64::try_from(lane.header.stable_key.len()).unwrap_or(u64::MAX),
                    );
                    total = total.saturating_add(lane_bytes);
                    largest = largest.max(lane_bytes);
                }
            }
        }
    }
    (total, largest)
}

fn requested_bytes<T>(count: u64) -> u64 {
    count.saturating_mul(u64::try_from(size_of::<T>()).unwrap_or(u64::MAX))
}

fn requested_hash_table_bytes<K, V>(entry_count: u64) -> u64 {
    if entry_count == 0 {
        return 0;
    }
    // 标准库不公开桶分配布局。预检为每个请求项预留八个桶，并额外计入每桶控制字节
    // 与一组尾部控制区，覆盖小表最小桶数和负载因子取整，而不依赖哈希表迭代顺序。
    // 真实生产基准仍须另报实际容量和进程内存，不能用本预算冒充测量。
    let bucket_bytes = u64::try_from(size_of::<(K, V)>())
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    entry_count
        .saturating_mul(8)
        .saturating_mul(bucket_bytes)
        .saturating_add(16)
}

fn count_to_usize(count: u64, limits: &crate::CompileLimits) -> Result<usize, DiagnosticBundle> {
    usize::try_from(count).map_err(|_| arena_overflow(ArenaKeyOverflow, limits, None))
}

fn arena_overflow(
    _: ArenaKeyOverflow,
    limits: &crate::CompileLimits,
    primary_span: Option<SourceSpan>,
) -> DiagnosticBundle {
    DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
        CompileLimitDimension::HirRecordCount,
        limits.value(CompileLimitDimension::HirRecordCount),
        u64::from(u32::MAX) + 1,
        primary_span,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CompilationUnitBuilder, CompileLimits, DiagnosticCode, DiagnosticPayload, LaneEdgeInput,
        LaneEdgeReference, SourceModuleHeader, SourceModuleHeaderInput, SyntheticModule,
        SyntheticModuleBuilder,
    };

    fn header(namespace: &str) -> SourceModuleHeader {
        SourceModuleHeader::new(
            SourceModuleHeaderInput {
                authoring_namespace_id: namespace,
                source_document_key: namespace,
                generator_build_id: "git:0123456789abcdef",
                parameters_and_inputs_digest: [0x11; 32],
                frontend_options_digest: [0x22; 32],
                random_seed: Some(42),
                provenance: "repository:laneflow",
            },
            &CompileLimits::p100_initial_v1(),
        )
        .unwrap()
    }

    fn module(
        namespace: &str,
        imports: &[&str],
        edges: &[(&str, &[LaneEdgeReference<'_>])],
    ) -> SyntheticModule {
        let limits = CompileLimits::p100_initial_v1();
        let mut builder = SyntheticModuleBuilder::new(header(namespace), &limits).unwrap();
        for import in imports {
            builder.add_import(import).unwrap();
        }
        for (key, successors) in edges {
            builder
                .add_lane_edge(LaneEdgeInput {
                    lane_edge_key: key,
                    length_meters: 12.5,
                    speed_limit_meters_per_second: 13.75,
                    successors,
                })
                .unwrap();
        }
        builder.finish().unwrap()
    }

    fn unit(modules: impl IntoIterator<Item = SyntheticModule>) -> CompilationUnit {
        let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v1());
        for module in modules {
            builder.add_synthetic_module(module).unwrap();
        }
        builder.build().unwrap()
    }

    #[test]
    fn hir_resolves_local_and_imported_lane_edge_references_to_typed_keys() {
        let base = module("city/base", &[], &[("edge-b", &[])]);
        let app_successors = [
            LaneEdgeReference::imported("city/base", "edge-b"),
            LaneEdgeReference::local("edge-c"),
        ];
        let app = module(
            "city/app",
            &["city/base"],
            &[("edge-c", &[]), ("edge-a", &app_successors)],
        );
        let unit = unit([app, base]);
        let hir = build_hir(&unit).unwrap();

        assert_eq!(hir.modules.len(), 2);
        assert_eq!(hir.modules[0].authoring_namespace_id.as_ref(), "city/base");
        assert_eq!(hir.modules[1].authoring_namespace_id.as_ref(), "city/app");
        assert_eq!(hir.imports.len(), 1);
        assert_eq!(hir.imports[0].target.raw(), 0);
        assert_eq!(hir.imports[0].source_span.source_document_key(), "city/app");
        assert_eq!(hir.modules[1].imports.start(), 0);
        assert_eq!(hir.modules[1].imports.len(), 1);
        assert_eq!(hir.lane_edges.len(), 3);
        assert_eq!(
            hir.lane_edges
                .iter()
                .map(|edge| edge.stable_key.as_ref())
                .collect::<Vec<_>>(),
            ["edge-b", "edge-a", "edge-c"]
        );
        let edge_a = &hir.lane_edges[1];
        let targets = hir.lane_edge_references[edge_a.successors.as_usize_range()]
            .iter()
            .map(|reference| reference.target.raw())
            .collect::<Vec<_>>();
        assert_eq!(targets, [2, 0]);
        assert!(hir.modules[0].imports.is_empty());
        assert_eq!(hir.hir_record_count, 16);
    }

    #[test]
    fn hir_reports_every_unknown_target_in_canonical_module_order() {
        let z_successors = [LaneEdgeReference::local("missing-z")];
        let a_successors = [LaneEdgeReference::local("missing-a")];
        let unit = unit([
            module("city/z", &[], &[("edge-z", &z_successors)]),
            module("city/a", &[], &[("edge-a", &a_successors)]),
        ]);
        let diagnostics = match build_hir(&unit) {
            Ok(_) => panic!("unknown targets must reject HIR construction"),
            Err(diagnostics) => diagnostics,
        };

        assert_eq!(diagnostics.diagnostics().len(), 2);
        assert_eq!(
            diagnostics
                .diagnostics()
                .iter()
                .map(|diagnostic| diagnostic.stable_key().unwrap())
                .collect::<Vec<_>>(),
            ["edge-a", "edge-z"]
        );
        assert!(diagnostics.diagnostics().iter().all(|diagnostic| {
            diagnostic.code() == DiagnosticCode::UnknownReferenceTarget
                && diagnostic.primary_span().is_some()
                && diagnostic.related_spans().len() == 1
        }));
    }

    #[test]
    fn hir_symbol_and_reference_order_ignore_declaration_insertion_order() {
        let successors = [
            LaneEdgeReference::local("edge-c"),
            LaneEdgeReference::local("edge-b"),
        ];
        let left = unit([module(
            "city/a",
            &[],
            &[("edge-a", &successors), ("edge-b", &[]), ("edge-c", &[])],
        )]);
        let right = unit([module(
            "city/a",
            &[],
            &[("edge-c", &[]), ("edge-a", &successors), ("edge-b", &[])],
        )]);
        let left = build_hir(&left).unwrap();
        let right = build_hir(&right).unwrap();

        let projection = |hir: &HirUnit| {
            hir.lane_edges
                .iter()
                .map(|edge| {
                    (
                        edge.stable_key.to_string(),
                        hir.lane_edge_references[edge.successors.as_usize_range()]
                            .iter()
                            .map(|reference| reference.target.raw())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(projection(&left), projection(&right));
        assert_eq!(
            left.lane_edges
                .iter()
                .map(|edge| edge.stable_id)
                .collect::<Vec<_>>(),
            right
                .lane_edges
                .iter()
                .map(|edge| edge.stable_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            projection(&left),
            [
                ("edge-a".into(), vec![1, 2]),
                ("edge-b".into(), vec![]),
                ("edge-c".into(), vec![]),
            ]
        );
    }

    #[test]
    fn hir_lane_edge_identity_uses_namespace_and_key_instead_of_dense_position() {
        let city_a = unit([module("city/a", &[], &[("edge-a", &[]), ("edge-b", &[])])]);
        let city_b = unit([module("city/b", &[], &[("edge-a", &[])])]);
        let city_a = build_hir(&city_a).unwrap();
        let city_b = build_hir(&city_b).unwrap();

        assert_ne!(
            city_a.lane_edges[0].stable_id,
            city_a.lane_edges[1].stable_id
        );
        assert_ne!(
            city_a.lane_edges[0].stable_id,
            city_b.lane_edges[0].stable_id
        );
        assert_eq!(
            city_a.lane_edges[0].stable_id.to_string(),
            format!(
                "lfid1_lane-edge_{:x}",
                city_a.lane_edges[0].stable_id.as_untyped()
            )
        );
    }

    #[test]
    fn hir_lane_edge_identity_ignores_non_identity_scalars_and_connections() {
        let baseline = unit([module("city/a", &[], &[("edge-a", &[]), ("edge-b", &[])])]);

        let limits = CompileLimits::p100_initial_v1();
        let mut changed = SyntheticModuleBuilder::new(header("city/a"), &limits).unwrap();
        changed
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-a",
                length_meters: 99.0,
                speed_limit_meters_per_second: 2.0,
                successors: &[LaneEdgeReference::local("edge-b")],
            })
            .unwrap();
        changed
            .add_lane_edge(LaneEdgeInput {
                lane_edge_key: "edge-b",
                length_meters: 1.0,
                speed_limit_meters_per_second: 1.0,
                successors: &[],
            })
            .unwrap();
        let changed = unit([changed.finish().unwrap()]);

        let baseline = build_hir(&baseline).unwrap();
        let changed = build_hir(&changed).unwrap();
        assert_eq!(baseline.lane_edges[0].stable_key.as_ref(), "edge-a");
        assert_eq!(changed.lane_edges[0].stable_key.as_ref(), "edge-a");
        assert_eq!(
            baseline.lane_edges[0].stable_id,
            changed.lane_edges[0].stable_id
        );
    }

    #[test]
    fn hir_checks_record_scratch_and_live_byte_limits_before_stage_allocation() {
        let mut unit = unit([module("city/a", &[], &[("edge-a", &[])])]);
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            3,
            u32::MAX,
            u32::MAX,
            u32::MAX,
        );
        let record_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR record limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(matches!(
            record_failure.diagnostics()[0].payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::HirRecordCount,
                limit: 3,
                observed: 4,
            }
        ));

        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            0,
            u32::MAX,
        );
        let scratch_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR scratch limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(
            scratch_failure
                .diagnostics()
                .iter()
                .any(|diagnostic| matches!(
                    diagnostic.payload(),
                    DiagnosticPayload::CompileLimitExceeded {
                        dimension: CompileLimitDimension::StageScratchBytes,
                        limit: 0,
                        observed,
                    } if *observed > 0
                ))
        );

        let source_live_bytes = u32::try_from(unit.controlled_live_bytes).unwrap();
        unit.limits = CompileLimits::p100_initial_v1().with_test_pipeline_limits(
            u32::MAX,
            u32::MAX,
            u32::MAX,
            source_live_bytes,
        );
        let live_failure = match build_hir(&unit) {
            Ok(_) => panic!("HIR live byte limit must fail closed"),
            Err(diagnostics) => diagnostics,
        };
        assert!(live_failure.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.payload(),
            DiagnosticPayload::CompileLimitExceeded {
                dimension: CompileLimitDimension::CompilerControlledLiveBytes,
                limit,
                observed,
            } if *limit == u64::from(source_live_bytes) && observed > limit
        )));
    }
}
