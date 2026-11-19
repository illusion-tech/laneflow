use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
use crate::SourceSpan;
use crate::arena::ArenaKey;
use crate::declaration::{
    ConflictZoneRegionDeclaration, RoadAlignmentDeclaration, TypedAstDeclaration,
};
use crate::diagnostic::DiagnosticCollector;
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::{
    CompileLimitDimension, CompileProfile, Diagnostic, DiagnosticBundle, SourceLocation,
    SourcePosition,
};

use super::descriptor::{SourceDocumentDescriptor, SourceModuleDescriptor};
#[cfg(test)]
use super::descriptor::{SourceDocumentOrigin, freeze_source_documents, source_document_digest};
#[cfg(test)]
use super::resources::size_bytes;
use super::resources::{AdmissionTotals, ModuleResourceCounts};
use super::synthetic::SyntheticModule;
use crate::ledger::{
    BoundedHashMap, BoundedVec, Ledger, LedgerError, LedgerStage, OutputGrant, PassBudget,
    PassLabel, Plan,
};

/// 区分编译单元来源文档登记序号的零尺寸标记。
pub(crate) enum SourceDocumentTag {}
/// 仅在同一次编译的来源模块描述符表内有效的致密序号。
pub(crate) type SourceDocumentOrdinal = ArenaKey<SourceDocumentTag>;

/// 一次显式模块导入的命名空间及其声明来源位置。
pub(crate) struct ImportRecord {
    pub(crate) namespace: Arc<str>,
    pub(crate) span: SourceLocation,
}

/// 官方前端完成受检构造后交给共同编译管线的 Typed AST 模块。
pub(crate) struct TypedAstModule {
    pub(crate) descriptor: SourceModuleDescriptor,
    pub(crate) declaration_span: SourceLocation,
    pub(crate) source_documents: Box<[SourceDocumentDescriptor]>,
    pub(crate) imports: Box<[ImportRecord]>,
    /// 只对已经执行 authoring numeric freeze 的官方来源存在；共同 HIR 在完整模块图上
    /// 校验所有此类模块使用同一对位置/方向档。
    pub(crate) geometry_profiles: Option<GeometryCompilationProfiles>,
    pub(crate) road_alignments: Box<[RoadAlignmentDeclaration]>,
    pub(crate) conflict_zone_regions: Box<[ConflictZoneRegionDeclaration]>,
    pub(crate) declarations: Box<[TypedAstDeclaration]>,
}

impl TypedAstModule {
    /// 返回本模块的来源模块描述符。
    pub(crate) const fn descriptor(&self) -> &SourceModuleDescriptor {
        &self.descriptor
    }

    /// 返回模块声明的来源位置。
    pub(crate) const fn declaration_span(&self) -> &SourceLocation {
        &self.declaration_span
    }

    /// 按声明顺序遍历导入命名空间及其来源位置。
    pub(crate) fn import_records(&self) -> impl ExactSizeIterator<Item = (&str, &SourceLocation)> {
        self.imports
            .iter()
            .map(|record| (record.namespace.as_ref(), &record.span))
    }
}

/// 描述符、文档、Typed AST 与准入资源计数不可分的内部载荷。
pub(crate) struct AdmittedOfficialModule {
    typed_ast: TypedAstModule,
    pub(super) resource_counts: ModuleResourceCounts,
    /// 准入时取得、随模块持有的载荷授予；准入前为空。
    payload: OutputGrant,
}

impl AdmittedOfficialModule {
    /// 组装准入模块载荷；要求来源文档已按文档键字节序规范排序。
    pub(crate) fn new(typed_ast: TypedAstModule, resource_counts: ModuleResourceCounts) -> Self {
        assert!(
            typed_ast.source_documents.windows(2).all(|pair| {
                pair[0].source_document_key.as_bytes() <= pair[1].source_document_key.as_bytes()
            }),
            "official frontends must canonically sort source documents before common admission"
        );
        Self {
            typed_ast,
            resource_counts,
            payload: OutputGrant::default(),
        }
    }

    /// 返回准入的 Typed AST 模块。
    pub(crate) const fn typed_ast(&self) -> &TypedAstModule {
        &self.typed_ast
    }
}

impl std::ops::Deref for AdmittedOfficialModule {
    type Target = TypedAstModule;

    fn deref(&self) -> &Self::Target {
        &self.typed_ast
    }
}

/// 仅用于证明共同接入不依赖 Synthetic 公开封装的第二官方前端测试封装。
#[cfg(test)]
pub(super) struct TestOfficialModule {
    pub(super) admitted: AdmittedOfficialModule,
}

/// 测试封装使用的单份来源文档输入。
#[cfg(test)]
pub(super) struct TestSourceDocument<'a> {
    pub(super) source_document_key: &'a str,
    pub(super) source_record: &'a [u8],
    pub(super) display_source: Option<&'a str>,
}

#[cfg(test)]
impl TestOfficialModule {
    /// 以追加来源文档的 Synthetic 模块构造测试官方模块。
    pub(super) fn from_synthetic_with_documents(
        module: SyntheticModule,
        documents: &[(&str, &[u8])],
    ) -> Self {
        let documents = documents
            .iter()
            .map(|(source_document_key, source_record)| TestSourceDocument {
                source_document_key,
                source_record,
                display_source: None,
            })
            .collect::<Vec<_>>();
        Self::from_synthetic_with_document_records(module, &documents)
    }

    /// 以文档记录构造测试官方模块，并同步累计追加文档的资源计数。
    pub(super) fn from_synthetic_with_document_records(
        module: SyntheticModule,
        documents: &[TestSourceDocument<'_>],
    ) -> Self {
        let mut admitted = module.admitted;
        let namespace = Arc::clone(&admitted.typed_ast.descriptor.authoring_namespace_id);
        let mut extra_source_bytes = 0_u64;
        let mut extra_string_bytes = 0_u64;
        let mut extra_string_items = 0_u64;
        let mut source_documents = admitted.typed_ast.source_documents.into_vec();
        let first = source_documents
            .pop()
            .expect("official module construction retains one primary source document");
        for document in documents {
            let source_record_byte_len = u32::try_from(document.source_record.len()).unwrap();
            source_documents.push(SourceDocumentDescriptor {
                source_document_key: Arc::from(document.source_document_key),
                source_document_digest: source_document_digest(document.source_record),
                source_record_byte_len,
                authoring_namespace_id: Arc::clone(&namespace),
                origin: SourceDocumentOrigin::test(document.display_source),
            });
            extra_source_bytes =
                extra_source_bytes.saturating_add(u64::from(source_record_byte_len));
            extra_string_bytes = extra_string_bytes
                .saturating_add(
                    u64::try_from(document.source_document_key.len()).unwrap_or(u64::MAX),
                )
                .saturating_add(
                    document
                        .display_source
                        .map_or(0, |source| u64::try_from(source.len()).unwrap_or(u64::MAX)),
                );
            extra_string_items = extra_string_items
                .saturating_add(1)
                .saturating_add(u64::from(document.display_source.is_some()));
        }
        let (source_documents, source_document_set_digest) =
            freeze_source_documents(&namespace, first, source_documents);
        admitted.typed_ast.descriptor.source_document_set_digest = source_document_set_digest;
        admitted.typed_ast.source_documents = source_documents;
        admitted.resource_counts.source_bytes = admitted
            .resource_counts
            .source_bytes
            .saturating_add(extra_source_bytes);
        admitted.resource_counts.string_item_count = admitted
            .resource_counts
            .string_item_count
            .saturating_add(extra_string_items);
        admitted.resource_counts.string_bytes = admitted
            .resource_counts
            .string_bytes
            .saturating_add(extra_string_bytes);
        admitted.resource_counts.controlled_live_bytes = admitted
            .resource_counts
            .controlled_live_bytes
            .saturating_add(extra_string_bytes)
            .saturating_add(size_bytes::<SourceDocumentDescriptor>(
                u64::try_from(documents.len()).unwrap_or(u64::MAX),
            ));
        admitted.resource_counts.admission_peak_live_bytes = admitted
            .resource_counts
            .admission_peak_live_bytes
            .max(admitted.resource_counts.controlled_live_bytes);
        let AdmittedOfficialModule {
            typed_ast,
            resource_counts,
            ..
        } = admitted;
        Self {
            admitted: AdmittedOfficialModule::new(typed_ast, resource_counts),
        }
    }

    /// 把首个车道边声明的来源位置改指到指定文档。
    pub(super) fn move_first_lane_edge_span_to(&mut self, source_document_key: &str) {
        let TypedAstDeclaration::LaneEdge(declaration) =
            &mut self.admitted.typed_ast.declarations[0]
        else {
            panic!("test wrapper expected first declaration to be LaneEdge");
        };
        declaration.header.span = SourceSpan::point(Arc::from(source_document_key), 41, 7).into();
    }

    /// 把模块声明的来源位置改指到指定文档。
    pub(super) fn move_module_declaration_span_to(&mut self, source_document_key: &str) {
        self.admitted.typed_ast.declaration_span =
            SourceSpan::point(Arc::from(source_document_key), 37, 5).into();
    }

    /// 构造来源文档未按文档键字节序排序的测试官方模块。
    pub(super) fn from_synthetic_with_unsorted_documents(
        module: SyntheticModule,
        documents: &[(&str, &[u8])],
    ) -> Self {
        let mut module = Self::from_synthetic_with_documents(module, documents);
        module.admitted.typed_ast.source_documents.swap(1, 2);
        let AdmittedOfficialModule {
            typed_ast,
            resource_counts,
            ..
        } = module.admitted;
        Self {
            admitted: AdmittedOfficialModule::new(typed_ast, resource_counts),
        }
    }

    /// 把首个车道边第一条后继引用的来源位置改指到指定文档。
    pub(super) fn move_first_lane_edge_successor_span_to(&mut self, source_document_key: &str) {
        let TypedAstDeclaration::LaneEdge(declaration) =
            &mut self.admitted.typed_ast.declarations[0]
        else {
            panic!("test wrapper expected first declaration to be LaneEdge");
        };
        declaration.successors[0].span =
            SourceSpan::point(Arc::from(source_document_key), 43, 9).into();
    }

    /// 把首个车道边各后继引用的来源位置逐一改指到指定文档。
    pub(super) fn move_first_lane_edge_successor_spans_to(
        &mut self,
        source_document_keys: &[&str],
    ) {
        let TypedAstDeclaration::LaneEdge(declaration) =
            &mut self.admitted.typed_ast.declarations[0]
        else {
            panic!("test wrapper expected first declaration to be LaneEdge");
        };
        assert_eq!(declaration.successors.len(), source_document_keys.len());
        for (index, (successor, source_document_key)) in declaration
            .successors
            .iter_mut()
            .zip(source_document_keys)
            .enumerate()
        {
            successor.span = SourceSpan::point(
                Arc::from(*source_document_key),
                43 + u32::try_from(index).unwrap(),
                9,
            )
            .into();
        }
    }

    /// 把首个停车设施声明的虚拟入口锚点来源位置改指到指定文档。
    pub(super) fn move_first_parking_virtual_anchor_span_to(&mut self, source_document_key: &str) {
        let declaration = self
            .admitted
            .typed_ast
            .declarations
            .iter_mut()
            .find_map(|declaration| match declaration {
                TypedAstDeclaration::ParkingFacility(declaration) => Some(declaration),
                _ => None,
            })
            .expect("test wrapper expected one ParkingFacility declaration");
        declaration.virtual_entries[0].lane_edge.span =
            SourceSpan::point(Arc::from(source_document_key), 47, 11).into();
    }

    /// 把信号控制器、相位状态与机动门信号控制引用的来源位置改指到指定文档。
    pub(super) fn move_signal_relation_spans_to(
        &mut self,
        controller_group_document: &str,
        phase_state_document: &str,
        gate_signal_document: &str,
    ) {
        let relation_offset = |stable_key: &str| match stable_key {
            "group-main" => 0,
            "group-release" => 1,
            other => panic!("unexpected test signal group {other}"),
        };
        let mut controller_group_count = 0_u32;
        let mut phase_state_count = 0_u32;
        let mut gate_signal_count = 0_u32;
        for declaration in &mut self.admitted.typed_ast.declarations {
            match declaration {
                TypedAstDeclaration::SignalController(controller) => {
                    for reference in &mut controller.signal_groups {
                        reference.span = SourceSpan::point(
                            Arc::from(controller_group_document),
                            51 + relation_offset(reference.declaration_key()),
                            3,
                        )
                        .into();
                        controller_group_count = controller_group_count.saturating_add(1);
                    }
                    for phase in &mut controller.phases {
                        for state in &mut phase.states {
                            state.signal_group.span = SourceSpan::point(
                                Arc::from(phase_state_document),
                                61 + relation_offset(state.signal_group.declaration_key()),
                                5,
                            )
                            .into();
                            phase_state_count = phase_state_count.saturating_add(1);
                        }
                    }
                }
                TypedAstDeclaration::ManeuverGate(gate) => {
                    if let crate::declaration::OwnedSignalControl::Group(reference) =
                        &mut gate.signal_control
                    {
                        reference.span = SourceSpan::point(
                            Arc::from(gate_signal_document),
                            71 + relation_offset(reference.declaration_key()),
                            7,
                        )
                        .into();
                        gate_signal_count = gate_signal_count.saturating_add(1);
                    }
                }
                _ => {}
            }
        }
        assert_eq!(controller_group_count, 2, "test module controller groups");
        assert_eq!(phase_state_count, 2, "test module phase states");
        assert_eq!(gate_signal_count, 2, "test module signal-controlled gates");
    }

    /// 把测试模块全部已声明关系引用的来源位置统一改指到指定文档。
    pub(super) fn move_authored_relation_spans_to(&mut self, source_document_key: &str) {
        self.move_signal_relation_spans_to(
            source_document_key,
            source_document_key,
            source_document_key,
        );

        let source_document_key: Arc<str> = source_document_key.into();
        let span = |line| {
            SourceLocation::from(SourceSpan::point(
                Arc::clone(&source_document_key),
                line,
                11,
            ))
        };
        let mut counts = [0_u32; 11];
        for declaration in &mut self.admitted.typed_ast.declarations {
            match declaration {
                TypedAstDeclaration::RoadCorridor(corridor) => {
                    for (index, element) in corridor.elements.iter_mut().enumerate() {
                        match element {
                            crate::declaration::OwnedCorridorElementReference::RoadSection(
                                reference,
                            ) => reference.span = span(81 + u32::try_from(index).unwrap()),
                            crate::declaration::OwnedCorridorElementReference::FacilityBand(
                                reference,
                            ) => reference.span = span(81 + u32::try_from(index).unwrap()),
                        }
                        counts[0] = counts[0].saturating_add(1);
                    }
                }
                TypedAstDeclaration::RoadSection(section) => {
                    for lane in &mut section.lanes {
                        lane.header.span = span(83);
                        counts[1] = counts[1].saturating_add(1);
                        if let Some(reference) = &mut lane.lane_group {
                            reference.span = span(84);
                            counts[2] = counts[2].saturating_add(1);
                        }
                    }
                }
                TypedAstDeclaration::Movement(movement) => {
                    movement.junction.span = span(85);
                    counts[3] = counts[3].saturating_add(1);
                }
                TypedAstDeclaration::ManeuverPath(path) => {
                    path.movement.span = span(86);
                    counts[4] = counts[4].saturating_add(1);
                }
                TypedAstDeclaration::ManeuverGate(gate) => {
                    gate.maneuver_path.span = span(87 + counts[5]);
                    counts[5] = counts[5].saturating_add(1);
                    gate.stop_line.span = span(89 + counts[6]);
                    counts[6] = counts[6].saturating_add(1);
                }
                TypedAstDeclaration::WaitingZone(waiting) => {
                    waiting.maneuver_path.span = span(91);
                    counts[7] = counts[7].saturating_add(1);
                }
                TypedAstDeclaration::ParkingSpace(space) => {
                    if let Some(reference) = &mut space.parking_facility {
                        reference.span = span(92);
                        counts[8] = counts[8].saturating_add(1);
                    }
                    space.entry.lane_edge.span = span(93);
                    counts[9] = counts[9].saturating_add(1);
                    space.exit.lane_edge.span = span(94);
                    counts[10] = counts[10].saturating_add(1);
                }
                _ => {}
            }
        }
        assert_eq!(
            counts,
            [2, 1, 1, 1, 1, 2, 2, 1, 1, 1, 1],
            "test module authored relation coverage",
        );
    }

    /// 强制覆盖指定维度的准入资源计数观测值。
    pub(super) fn force_resource_count(&mut self, dimension: CompileLimitDimension, observed: u64) {
        let counts = &mut self.admitted.resource_counts;
        match dimension {
            CompileLimitDimension::DeclarationCount => counts.declaration_count = observed,
            CompileLimitDimension::StableEntityCount => counts.stable_entity_count = observed,
            CompileLimitDimension::TypedAstRecordCount => counts.typed_ast_record_count = observed,
            CompileLimitDimension::ReferenceCount => counts.reference_count = observed,
            CompileLimitDimension::RelationOccurrenceCount => {
                counts.relation_occurrence_count = observed;
            }
            CompileLimitDimension::IdentityFieldOccurrenceCount => {
                counts.identity_field_occurrence_count = observed;
            }
            CompileLimitDimension::SymbolCount => counts.symbol_count = observed,
            CompileLimitDimension::StringItemCount => counts.string_item_count = observed,
            CompileLimitDimension::TotalStringBytes => counts.string_bytes = observed,
            CompileLimitDimension::ManeuverGateCount => counts.maneuver_gate_count = observed,
            CompileLimitDimension::WaitingZoneCount => counts.waiting_zone_count = observed,
            CompileLimitDimension::GeometryPointCount => counts.geometry_point_count = observed,
            _ => panic!("test dimension is not represented by ModuleResourceCounts"),
        }
    }
}

const UNBOUND_SOURCE_DOCUMENT_ORDINAL: u32 = u32::MAX;

#[derive(Clone, Copy)]
pub(super) struct SourceDocumentBinding {
    owner_module_ordinal: u32,
    source_document_ordinal: u32,
}

impl SourceDocumentBinding {
    fn pending(owner_module_index: usize) -> Self {
        Self {
            owner_module_ordinal: u32::try_from(owner_module_index)
                .expect("compile limits bound module indexes to u32"),
            source_document_ordinal: UNBOUND_SOURCE_DOCUMENT_ORDINAL,
        }
    }

    fn freeze(&mut self, owner_module_ordinal: u32, source_document_ordinal: u32) {
        self.owner_module_ordinal = owner_module_ordinal;
        self.source_document_ordinal = source_document_ordinal;
    }

    fn pending_owner_module_index(self) -> usize {
        usize::try_from(self.owner_module_ordinal)
            .expect("u32 module indexes fit usize on supported targets")
    }
}

/// 构建期文档索引；值中保存加入顺序的模块下标，不能用于源映射解析。
struct PendingSourceDocumentIndex {
    bindings: BoundedHashMap<Arc<str>, SourceDocumentBinding>,
}

impl PendingSourceDocumentIndex {
    #[inline]
    fn get(&self, source_document_key: &str) -> Option<SourceDocumentBinding> {
        self.bindings.get(source_document_key).copied()
    }

    #[inline]
    fn insert(
        &mut self,
        source_document_key: Arc<str>,
        binding: SourceDocumentBinding,
    ) -> Result<Option<SourceDocumentBinding>, LedgerError> {
        self.bindings.insert(source_document_key, binding)
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    fn freeze(
        &mut self,
        modules: &[(usize, AdmittedOfficialModule)],
        expected_document_count: u64,
    ) {
        let mut source_document_ordinal = 0_u32;
        for (module_ordinal, (_, module)) in modules.iter().enumerate() {
            let module_ordinal = u32::try_from(module_ordinal)
                .expect("compile limits bound canonical module ordinals to u32");
            for document in &module.source_documents {
                self.bindings
                    .get_mut(document.source_document_key())
                    .expect("admission indexed every official source document")
                    .freeze(module_ordinal, source_document_ordinal);
                source_document_ordinal = source_document_ordinal
                    .checked_add(1)
                    .expect("compile limits bound source document ordinals to u32");
            }
        }
        debug_assert_eq!(u64::from(source_document_ordinal), expected_document_count);
    }
}

/// 冻结期文档索引；所有值均已换算为规范模块序号和全局文档序号。
struct FrozenSourceDocumentIndex {
    bindings: HashMap<Arc<str>, SourceDocumentBinding>,
}

impl FrozenSourceDocumentIndex {
    #[inline]
    fn resolve(&self, source_document_key: &str) -> Option<ResolvedSourceDocument> {
        let binding = self.bindings.get(source_document_key).copied()?;
        assert_ne!(
            binding.source_document_ordinal, UNBOUND_SOURCE_DOCUMENT_ORDINAL,
            "compilation unit source document bindings must be frozen"
        );
        Some(ResolvedSourceDocument {
            owner_module_ordinal: binding.owner_module_ordinal,
            source_document_ordinal: SourceDocumentOrdinal::from_raw(
                binding.source_document_ordinal,
            ),
        })
    }
}

/// 只暴露 HIR 与源映射阶段所需事实的解析结果，不泄漏索引内部状态。
#[derive(Clone, Copy)]
pub(crate) struct ResolvedSourceDocument {
    owner_module_ordinal: u32,
    source_document_ordinal: SourceDocumentOrdinal,
}

impl ResolvedSourceDocument {
    /// 返回拥有该文档的模块序号。
    pub(crate) const fn owner_module_ordinal(self) -> u32 {
        self.owner_module_ordinal
    }

    /// 返回编译单元内的全局来源文档序号。
    pub(crate) const fn source_document_ordinal(self) -> SourceDocumentOrdinal {
        self.source_document_ordinal
    }
}

/// 已核对模块归属并解析到编译单元文档表的紧凑来源位置。
///
/// HIR/MIR 关系记录携带本值，避免在后续热循环中继续持有或比较文档键字符串。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResolvedSourceLocation {
    Text {
        source_document_ordinal: SourceDocumentOrdinal,
        start: SourcePosition,
        end: SourcePosition,
    },
    RoadEditing {
        source_document_ordinal: SourceDocumentOrdinal,
        location: crate::RoadEditingSourceLocation,
    },
}

/// 创建空 builder 的编译遍：只建立零容量的模块表与索引。
const BUILDER_INIT_PASS: PassLabel = PassLabel::new(LedgerStage::Admission, "builder");
/// 一次模块准入调用：候选载荷与 builder 模块表、索引的扩容。
const ADMIT_MODULE_PASS: PassLabel = PassLabel::new(LedgerStage::Admission, "admit-module");
/// 冻结编译单元：导入闭包、规范拓扑序与重排。
const BUILD_PASS: PassLabel = PassLabel::new(LedgerStage::Admission, "build");

fn count(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// 准入调用的计划：候选载荷、本次扩容后的模块表与两份索引。
///
/// 具体官方前端构造候选期间的峰值不含既有 builder；它与本次准入互斥，取峰值。
pub(super) fn admission_plan(
    current: AdmissionTotals,
    next: AdmissionTotals,
    candidate_retained_live: u64,
    candidate_frontend_peak: u64,
) -> Plan {
    let mut retained = Plan::of(candidate_retained_live);
    if next.module_slot_capacity > current.module_slot_capacity {
        let slots = count(next.module_slot_capacity);
        retained = retained
            .array::<AdmittedOfficialModule>(slots)
            .hash_table::<Arc<str>, usize>(slots);
    }
    if next.source_document_slot_capacity > current.source_document_slot_capacity {
        retained = retained.hash_table::<Arc<str>, SourceDocumentBinding>(count(
            next.source_document_slot_capacity,
        ));
    }
    retained.max(Plan::of(candidate_frontend_peak))
}

/// 冻结编译单元的计划峰值。
///
/// 规范下标与诊断缓冲贯穿全程；拓扑排序、环路诊断与重排三段互斥，取峰值。重排期间
/// 拓扑序、规范名次、重排元组表与最终 Typed AST 表共存。
pub(super) fn build_plan(totals: AdmissionTotals, diagnostic_limit: u64) -> Plan {
    let modules = count(totals.module_count);
    let imports = count(totals.import_edge_count);
    let topology = Plan::ZERO
        .array::<usize>(modules)
        .array::<usize>(modules.saturating_add(1))
        .array::<usize>(modules)
        .array::<usize>(imports)
        .array::<usize>(modules)
        .array::<usize>(modules);
    let reorder = Plan::ZERO
        .array::<usize>(modules)
        .array::<usize>(modules)
        .array::<(usize, AdmittedOfficialModule)>(modules)
        .array::<TypedAstModule>(modules);
    Plan::ZERO
        .array::<usize>(modules)
        .array::<Diagnostic>(count(diagnostic_limit))
        .plus(topology.max(cycle_plan(modules, imports)).max(reorder))
}

/// 环路诊断的工作集上界：Tarjan 状态、强连通分量、递归路径上的依赖列表、逐分量
/// 见证路径与环路列表同时存续的保守和；诊断载荷按模块数计入。
fn cycle_plan(modules: usize, imports: usize) -> Plan {
    Plan::ZERO
        .array::<usize>(modules)
        .array::<Option<usize>>(modules)
        .array::<usize>(modules)
        .array::<usize>(modules)
        .array::<bool>(modules)
        .array::<usize>(modules)
        .array::<(usize, usize)>(modules)
        .array::<usize>(imports)
        .array::<bool>(modules)
        .array::<bool>(modules)
        .array::<usize>(modules)
        .array::<usize>(imports)
        .array::<usize>(modules)
        .array::<(usize, usize)>(modules)
        .array::<u32>(modules)
        .array::<&str>(modules)
        .array::<SourceLocation>(modules)
}

/// 准入账本拒绝计划时的诊断：维度为 `CompilerControlledLiveBytes`，观测值为 `L + R + P`。
fn live_budget_error(
    error: LedgerError,
    primary_span: Option<SourceLocation>,
    namespace: Option<&str>,
) -> DiagnosticBundle {
    match error {
        LedgerError::BudgetExceeded {
            requested, limit, ..
        } => DiagnosticBundle::single(Diagnostic::compile_limit_exceeded_at(
            CompileLimitDimension::CompilerControlledLiveBytes,
            limit,
            requested,
            primary_span,
            namespace.map(Into::into),
        )),
        other => other.into(),
    }
}

/// 只通过具体且封闭的官方前端入口接收受检来源模块的编译单元构建器。
///
/// 模块可以按任意顺序加入；[`CompilationUnitBuilder::build`] 会验证导入闭包与循环，
/// 再冻结规范依赖顺序。构建器不会访问文件系统或自动发现模块。
///
/// 共同 Typed AST 接入面保持包内私有，外部调用方只能使用具体官方入口：
///
/// ```compile_fail
/// use laneflow_compiler::{CompilationUnitBuilder, CompileProfile};
/// let mut builder = CompilationUnitBuilder::new(CompileProfile::single_network_10k_stable_entities());
/// let _generic_admission = builder.add_module;
/// ```
pub struct CompilationUnitBuilder {
    limits: CompileProfile,
    /// 准入账本：跨越 builder 生命周期，持有已准入模块、模块表与索引的存续。
    ledger: Ledger,
    pub(super) modules: BoundedVec<AdmittedOfficialModule>,
    pub(super) module_index: BoundedHashMap<Arc<str>, usize>,
    source_document_index: PendingSourceDocumentIndex,
    pub(super) totals: AdmissionTotals,
}

/// 扩容后的模块表与模块索引。
type GrownModuleStorage = (
    BoundedVec<AdmittedOfficialModule>,
    BoundedHashMap<Arc<str>, usize>,
);

/// 已完整校验、尚未写入构建器的单模块准入事务；扩容后的存储已经取得授予。
struct AdmissionPlan {
    module: AdmittedOfficialModule,
    next_totals: AdmissionTotals,
    grown_modules: Option<GrownModuleStorage>,
    grown_documents: Option<BoundedHashMap<Arc<str>, SourceDocumentBinding>>,
}

impl CompilationUnitBuilder {
    /// 用同一份显式资源配置档建立空编译单元构建器。
    #[must_use]
    pub fn new(limits: CompileProfile) -> Self {
        const EMPTY: &str = "zero-capacity builder storage needs no budget";
        let ledger = Ledger::new(limits.operation_budget());
        let pass = ledger
            .admit_pass(BUILDER_INIT_PASS, Plan::ZERO)
            .expect(EMPTY);
        let modules = pass.vec(0).expect(EMPTY);
        let module_index = pass.hash_map(0).expect(EMPTY);
        let source_document_index = PendingSourceDocumentIndex {
            bindings: pass.hash_map(0).expect(EMPTY),
        };
        pass.finish(0).expect(EMPTY);
        Self {
            limits,
            ledger,
            modules,
            module_index,
            source_document_index,
            totals: AdmissionTotals::default(),
        }
    }

    /// 判断构建期来源文档索引是否为空。
    #[cfg(test)]
    pub(super) fn source_document_index_is_empty(&self) -> bool {
        self.source_document_index.is_empty()
    }

    /// 替换构建器使用的限额配置档。
    #[cfg(test)]
    pub(super) fn set_test_limits(&mut self, limits: CompileProfile) {
        self.ledger.set_test_budget(
            limits
                .operation_budget()
                .get(crate::OperationResourceDimension::CompilerControlledLiveBytes),
        );
        self.limits = limits;
    }

    /// 冻结编译单元时准入计划所核对的 `L + P`。
    #[cfg(test)]
    pub(super) fn build_peak_live_bytes(&self) -> u64 {
        let plan = build_plan(
            self.totals,
            self.limits
                .source_limits()
                .get(crate::SourceProcessingDimension::DiagnosticCount),
        );
        self.ledger
            .live()
            .saturating_add(plan.bytes().unwrap_or(u64::MAX))
    }

    /// 原子加入一个已经由官方前端完成受检构造的模块。
    ///
    /// # Errors
    ///
    /// 当 authoring namespace 或 `sourceDocumentKey` 与已加入模块重复，或加入后的模块、
    /// 来源字节、声明、引用、字符串及存续内存等累计维度超过配置档时失败。失败不会
    /// 改变构建器的索引与计数，但 `module` 按值传入并会被释放；重试时需要重新构造该
    /// 模块。
    pub fn add_synthetic_module(
        &mut self,
        module: SyntheticModule,
    ) -> Result<&mut Self, DiagnosticBundle> {
        self.admit_official_module(module.admitted)
    }

    /// 测试入口：原子加入一个测试官方模块。
    #[cfg(test)]
    pub(super) fn add_test_official_module(
        &mut self,
        module: TestOfficialModule,
    ) -> Result<&mut Self, DiagnosticBundle> {
        self.admit_official_module(module.admitted)
    }

    /// 校验并原子准入一个官方模块；失败不改变构建器索引与计数。
    #[inline]
    pub(crate) fn admit_official_module(
        &mut self,
        module: AdmittedOfficialModule,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let plan = self.prepare_admission(module)?;
        self.commit_admission(plan);
        Ok(self)
    }

    /// 返回构建器使用的限额配置档。
    pub(crate) const fn road_editing_limits(&self) -> &CompileProfile {
        &self.limits
    }

    /// 返回已准入的来源字节累计量。
    pub(crate) const fn road_editing_source_bytes_already_admitted(&self) -> u64 {
        self.totals.source_bytes_total
    }

    /// 返回已准入的 Typed AST 记录累计数。
    pub(crate) const fn road_editing_typed_ast_records_already_admitted(&self) -> u64 {
        self.totals.typed_ast_record_count
    }

    /// 返回几何点维度剩余可用的限额额度。
    pub(crate) fn road_editing_remaining_geometry_points(&self) -> u64 {
        self.limits
            .canonical_capacity()
            .get(laneflow_static_contract::CanonicalNetworkDimension::GeometryPointCount)
            .saturating_sub(self.totals.geometry_point_count)
    }

    /// 返回指定限额维度已准入的累计观测值。
    /// 共同准入已经持有的来源计数，按来源维度读取。
    pub(crate) fn admitted_source_usage(&self, dimension: crate::SourceProcessingDimension) -> u64 {
        match dimension {
            crate::SourceProcessingDimension::ModuleCount => self.totals.module_count,
            crate::SourceProcessingDimension::SourceDocumentCount => {
                self.totals.source_document_count
            }
            crate::SourceProcessingDimension::ImportEdgeCount => self.totals.import_edge_count,
            crate::SourceProcessingDimension::SourceBytesTotal => self.totals.source_bytes_total,
            crate::SourceProcessingDimension::DeclarationCount => self.totals.declaration_count,
            crate::SourceProcessingDimension::TypedAstRecordCount => {
                self.totals.typed_ast_record_count
            }
            crate::SourceProcessingDimension::ReferenceCount => self.totals.reference_count,
            crate::SourceProcessingDimension::RelationOccurrenceCount => {
                self.totals.relation_occurrence_count
            }
            crate::SourceProcessingDimension::IdentityFieldOccurrenceCount => {
                self.totals.identity_field_occurrence_count
            }
            crate::SourceProcessingDimension::SymbolCount => self.totals.symbol_count,
            crate::SourceProcessingDimension::StringItemCount => self.totals.string_item_count,
            crate::SourceProcessingDimension::TotalStringBytes => self.totals.string_bytes,
            crate::SourceProcessingDimension::SourceBytesPerModule
            | crate::SourceProcessingDimension::SingleStringBytes
            | crate::SourceProcessingDimension::DiagnosticCount
            | crate::SourceProcessingDimension::HirRecordCount
            | crate::SourceProcessingDimension::MirRecordCount
            | crate::SourceProcessingDimension::LirRecordCount => 0,
        }
    }
    /// 共同准入阶段已知的规范数量；规范关系须由最终 LIR 独立统计。
    pub(crate) fn admitted_canonical_lower_bound(
        &self,
    ) -> laneflow_static_contract::CanonicalNetworkUsage {
        laneflow_static_contract::CanonicalNetworkUsage::new(
            self.totals.stable_entity_count,
            self.totals.maneuver_gate_count,
            self.totals.waiting_zone_count,
            0,
            self.totals.geometry_point_count,
        )
    }
    /// 构建器当前控制的存续字节。
    pub(crate) fn admitted_controlled_live_bytes(&self) -> u64 {
        self.ledger.live()
    }

    #[cfg(test)]
    pub(crate) fn already_admitted(&self, dimension: CompileLimitDimension) -> u64 {
        match dimension {
            CompileLimitDimension::ModuleCount => self.totals.module_count,
            CompileLimitDimension::SourceDocumentCount => self.totals.source_document_count,
            CompileLimitDimension::ImportEdgeCount => self.totals.import_edge_count,
            CompileLimitDimension::SourceBytesTotal => self.totals.source_bytes_total,
            CompileLimitDimension::DeclarationCount => self.totals.declaration_count,
            CompileLimitDimension::StableEntityCount => self.totals.stable_entity_count,
            CompileLimitDimension::TypedAstRecordCount => self.totals.typed_ast_record_count,
            CompileLimitDimension::ReferenceCount => self.totals.reference_count,
            CompileLimitDimension::RelationOccurrenceCount => self.totals.relation_occurrence_count,
            CompileLimitDimension::IdentityFieldOccurrenceCount => {
                self.totals.identity_field_occurrence_count
            }
            CompileLimitDimension::SymbolCount => self.totals.symbol_count,
            CompileLimitDimension::StringItemCount => self.totals.string_item_count,
            CompileLimitDimension::TotalStringBytes => self.totals.string_bytes,
            CompileLimitDimension::ManeuverGateCount => self.totals.maneuver_gate_count,
            CompileLimitDimension::WaitingZoneCount => self.totals.waiting_zone_count,
            CompileLimitDimension::GeometryPointCount => self.totals.geometry_point_count,
            CompileLimitDimension::CompilerControlledLiveBytes => self.ledger.live(),
            CompileLimitDimension::SourceBytesPerModule
            | CompileLimitDimension::SingleStringBytes
            | CompileLimitDimension::DiagnosticCount
            | CompileLimitDimension::HirRecordCount
            | CompileLimitDimension::MirRecordCount
            | CompileLimitDimension::LirRecordCount
            | CompileLimitDimension::RetainedCapacityBytes
            | CompileLimitDimension::PolicyValidationWorkUnits
            | CompileLimitDimension::CanonicalRelationCount => 0,
        }
    }

    #[inline]
    fn prepare_admission(
        &self,
        mut module: AdmittedOfficialModule,
    ) -> Result<AdmissionPlan, DiagnosticBundle> {
        let namespace = module.descriptor.authoring_namespace_id.as_ref();
        if let Some(existing_index) = self.module_index.get(namespace).copied() {
            return Err(DiagnosticBundle::single(
                Diagnostic::duplicate_module_namespace(
                    namespace,
                    module.declaration_span.clone(),
                    self.modules[existing_index].declaration_span.clone(),
                ),
            ));
        }

        let document_count = u64::try_from(module.source_documents.len()).unwrap_or(u64::MAX);
        assert_ne!(
            document_count, 0,
            "official logical module construction guarantees a non-empty document set"
        );
        if let Some(pair) = module.source_documents.windows(2).find(|pair| {
            pair[0].source_document_key.as_ref() == pair[1].source_document_key.as_ref()
        }) {
            let source_document_key = pair[0].source_document_key.as_ref();
            return Err(DiagnosticBundle::single(
                Diagnostic::duplicate_source_document_key(
                    source_document_key,
                    module.declaration_span.clone(),
                    module.declaration_span.clone(),
                ),
            ));
        }
        for document in &module.source_documents {
            let source_document_key = document.source_document_key.as_ref();
            if let Some(existing_binding) = self.source_document_index.get(source_document_key) {
                let existing_index = existing_binding.pending_owner_module_index();
                return Err(DiagnosticBundle::single(
                    Diagnostic::duplicate_source_document_key(
                        source_document_key,
                        module.declaration_span.clone(),
                        self.modules[existing_index].declaration_span.clone(),
                    ),
                ));
            }
        }

        let candidate_frontend_peak = module.resource_counts.admission_peak_live_bytes;
        let candidate_retained_live = module.resource_counts.controlled_live_bytes;
        let mut next_totals = self.totals.candidate_after(
            document_count,
            u64::try_from(module.imports.len()).unwrap_or(u64::MAX),
            &module.resource_counts,
        );
        debug_assert_eq!(
            next_totals.module_count,
            u64::try_from(self.modules.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1)
        );
        if let Some(violation) = next_totals
            .limit_violations(&self.limits)
            .into_iter()
            .flatten()
            .next()
        {
            let crate::limits::ResourceLimitExceeded {
                dimension,
                limit,
                observed,
            } = violation;
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded_at(
                    dimension,
                    limit,
                    observed,
                    Some(module.declaration_span.clone()),
                    Some(namespace.into()),
                ),
            ));
        }

        // 候选载荷、扩容后的模块表与索引都在本次准入调用内取得授予；提交阶段只移动
        // 已授予的存储，不再发生可能失败的分配。
        let plan = admission_plan(
            self.totals,
            next_totals,
            candidate_retained_live,
            candidate_frontend_peak,
        );
        let observed_live = self
            .ledger
            .live()
            .saturating_add(plan.bytes().unwrap_or(u64::MAX));
        let pass = self
            .ledger
            .admit_pass(ADMIT_MODULE_PASS, plan)
            .map_err(|error| {
                live_budget_error(
                    error,
                    Some(module.declaration_span.clone()),
                    Some(namespace),
                )
            })?;
        let source_document_limit = self
            .limits
            .source_limits()
            .get(crate::SourceProcessingDimension::SourceDocumentCount);
        if next_totals.source_document_count > source_document_limit {
            let limit = source_document_limit;
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_limit_exceeded_at(
                    CompileLimitDimension::SourceDocumentCount,
                    limit,
                    next_totals.source_document_count,
                    Some(module.declaration_span.clone()),
                    Some(namespace.into()),
                ),
            ));
        }
        next_totals.admission_peak_live_bytes =
            next_totals.admission_peak_live_bytes.max(observed_live);

        // 候选载荷尚未逐项授予：具体前端的计量值作为一笔授予随模块持有（资源账本设计
        // 第 5 节道路编辑候选模块存储另行迁移）。
        module.payload = OutputGrant::new(pass.grant_bytes(candidate_retained_live)?);
        let mut output_bytes = module.payload.bytes();
        let grown_modules = if next_totals.module_slot_capacity > self.totals.module_slot_capacity {
            let slots = count(next_totals.module_slot_capacity);
            let modules = pass.vec(slots)?;
            let index = pass.hash_map(slots)?;
            output_bytes = output_bytes
                .saturating_add(modules.granted_bytes())
                .saturating_add(index.granted_bytes());
            Some((modules, index))
        } else {
            None
        };
        let grown_documents = if next_totals.source_document_slot_capacity
            > self.totals.source_document_slot_capacity
        {
            let documents = pass.hash_map(count(next_totals.source_document_slot_capacity))?;
            output_bytes = output_bytes.saturating_add(documents.granted_bytes());
            Some(documents)
        } else {
            None
        };
        pass.finish(output_bytes)?;

        Ok(AdmissionPlan {
            module,
            next_totals,
            grown_modules,
            grown_documents,
        })
    }

    #[inline]
    fn commit_admission(&mut self, plan: AdmissionPlan) {
        const RESERVED: &str = "admission reserved builder capacity before commit";
        let AdmissionPlan {
            module,
            next_totals,
            grown_modules,
            grown_documents,
        } = plan;
        if let Some((mut modules, mut index)) = grown_modules {
            modules.try_extend(self.modules.drain()).expect(RESERVED);
            for (namespace, position) in self.module_index.drain() {
                index.insert(namespace, position).expect(RESERVED);
            }
            self.modules = modules;
            self.module_index = index;
        }
        if let Some(mut documents) = grown_documents {
            for (key, binding) in self.source_document_index.bindings.drain() {
                documents.insert(key, binding).expect(RESERVED);
            }
            self.source_document_index.bindings = documents;
        }
        let namespace = Arc::clone(&module.descriptor.authoring_namespace_id);
        let module_index = self.modules.len();
        self.modules.push(module).expect(RESERVED);
        self.module_index
            .insert(namespace, module_index)
            .expect(RESERVED);
        for document in &self.modules[module_index].source_documents {
            let previous = self
                .source_document_index
                .insert(
                    document.source_document_key_arc(),
                    SourceDocumentBinding::pending(module_index),
                )
                .expect(RESERVED);
            debug_assert!(
                previous.is_none(),
                "admission plan guaranteed unique document keys"
            );
        }
        self.totals = next_totals;
        debug_assert_eq!(
            u64::try_from(self.modules.capacity()).unwrap_or(u64::MAX),
            self.totals.module_slot_capacity
        );
    }

    /// 验证完整导入图并冻结依赖优先的规范模块顺序。
    ///
    /// 无依赖或同时就绪的模块按 authoring namespace 字节序打破平局；该顺序成为后续
    /// HIR 及诊断排序的模块轴，与调用方加入顺序无关。
    ///
    /// # Errors
    ///
    /// 任一显式导入没有对应模块，或导入图包含一个或多个循环时，返回有界、规范有序
    /// 诊断且不返回部分 [`CompilationUnit`]。该方法无论成功或失败都会消费构建器。
    pub fn build(mut self) -> Result<CompilationUnit, DiagnosticBundle> {
        let diagnostic_limit = self
            .limits
            .source_limits()
            .get(crate::SourceProcessingDimension::DiagnosticCount);
        let primary_module = self.modules.iter().min_by(|left, right| {
            left.descriptor
                .authoring_namespace_id
                .cmp(&right.descriptor.authoring_namespace_id)
        });
        let primary_span = primary_module.map(|module| module.declaration_span.clone());
        let primary_namespace: Option<Arc<str>> =
            primary_module.map(|module| Arc::clone(&module.descriptor.authoring_namespace_id));
        let plan = build_plan(self.totals, diagnostic_limit);
        let build_peak_live_bytes = self
            .ledger
            .live()
            .saturating_add(plan.bytes().unwrap_or(u64::MAX));
        let pass = self.ledger.admit_pass(BUILD_PASS, plan).map_err(|error| {
            live_budget_error(error, primary_span, primary_namespace.as_deref())
        })?;

        let mut diagnostics = DiagnosticCollector::new(diagnostic_limit);
        let mut canonical_indices = pass.collect(0..self.modules.len())?;
        canonical_indices.sort_unstable_by(|left, right| {
            self.modules[*left]
                .descriptor
                .authoring_namespace_id
                .cmp(&self.modules[*right].descriptor.authoring_namespace_id)
        });
        for (order, module_index) in canonical_indices.iter().copied().enumerate() {
            let module = &self.modules[module_index];
            for import in &module.imports {
                if !self.module_index.contains_key(import.namespace.as_ref()) {
                    let mut diagnostic =
                        Diagnostic::unknown_import(&import.namespace, import.span.clone());
                    diagnostic.set_canonical_module_order(u32::try_from(order).unwrap_or(u32::MAX));
                    diagnostics.push(diagnostic);
                }
            }
        }
        if !diagnostics.is_empty() {
            return Err(diagnostics.finish());
        }

        let order = match canonical_topological_order(&pass, &self.modules, &self.module_index)? {
            Ok(order) => order,
            Err(cycles) => {
                let mut canonical_order_by_index =
                    pass.collect(std::iter::repeat_n(0_u32, self.modules.len()))?;
                for (order, module_index) in canonical_indices.iter().copied().enumerate() {
                    canonical_order_by_index[module_index] =
                        u32::try_from(order).unwrap_or(u32::MAX);
                }
                for cycle in cycles.iter() {
                    let namespaces: Vec<_> = cycle
                        .iter()
                        .map(|index| {
                            self.modules[*index]
                                .descriptor
                                .authoring_namespace_id
                                .as_ref()
                        })
                        .collect();
                    let spans: Vec<_> = cycle
                        .iter()
                        .enumerate()
                        .filter_map(|(position, module_index)| {
                            let next_index = cycle[(position + 1) % cycle.len()];
                            let next_namespace = self.modules[next_index]
                                .descriptor
                                .authoring_namespace_id
                                .as_ref();
                            self.modules[*module_index]
                                .imports
                                .iter()
                                .find(|import| import.namespace.as_ref() == next_namespace)
                                .map(|import| import.span.clone())
                        })
                        .collect();
                    let mut diagnostic =
                        Diagnostic::import_cycle(&namespaces, spans.into_boxed_slice());
                    if let Some(first_index) = cycle.first().copied() {
                        diagnostic
                            .set_canonical_module_order(canonical_order_by_index[first_index]);
                    }
                    diagnostics.push(diagnostic);
                }
                return Err(diagnostics.finish());
            }
        };
        drop(canonical_indices);

        let mut canonical_rank = pass.collect(std::iter::repeat_n(0_usize, order.len()))?;
        for (rank, index) in order.iter().copied().enumerate() {
            canonical_rank[index] = rank;
        }
        drop(order);
        let mut modules = pass.collect(self.modules.drain().enumerate())?;
        modules.sort_unstable_by_key(|(original_index, _)| canonical_rank[*original_index]);
        drop(canonical_rank);
        self.source_document_index
            .freeze(&modules, self.totals.source_document_count);
        // 模块载荷授予属于 builder 的准入存续，在返回点逐项转交宿主。
        let mut payload_bytes = 0_u64;
        let typed_ast = pass.collect(modules.drain().map(|(_, module)| {
            let AdmittedOfficialModule {
                typed_ast,
                mut payload,
                ..
            } = module;
            payload_bytes = payload_bytes.saturating_add(payload.bytes());
            payload.transfer_to_host();
            typed_ast
        }))?;
        drop(modules);
        let typed_ast = typed_ast.freeze()?;
        let module_table_bytes = typed_ast.granted_bytes();
        pass.finish(module_table_bytes)?;
        let CompilationUnitBuilder {
            limits,
            ledger,
            modules: admitted_modules,
            module_index,
            source_document_index,
            totals,
        } = self;
        drop(admitted_modules);
        drop(module_index);
        let source_document_index_bytes = source_document_index.bindings.granted_bytes();
        let source_document_index = FrozenSourceDocumentIndex {
            bindings: source_document_index.bindings.into_host(),
        };
        let modules = typed_ast.into_host();
        let report = ledger.close()?;
        Ok(CompilationUnit {
            limits,
            modules,
            source_document_index,
            source_document_count: totals.source_document_count,
            import_edge_count: totals.import_edge_count,
            declaration_count: totals.declaration_count,
            stable_entity_count: totals.stable_entity_count,
            reference_count: totals.reference_count,
            relation_occurrence_count: totals.relation_occurrence_count,
            identity_field_occurrence_count: totals.identity_field_occurrence_count,
            symbol_count: totals.symbol_count,
            maneuver_gate_count: totals.maneuver_gate_count,
            waiting_zone_count: totals.waiting_zone_count,
            controlled_live_bytes: payload_bytes
                .saturating_add(source_document_index_bytes)
                .saturating_add(module_table_bytes),
            admission_peak_live_bytes: totals
                .admission_peak_live_bytes
                .max(build_peak_live_bytes)
                .max(report.peak_committed),
        })
    }
}

/// 规范模块顺序已冻结的原子编译输入。
///
/// 构造完成后，全部导入目标存在、导入图无环，并且 `modules` 按依赖优先的规范顺序
/// 排列。类型字段私有，后续阶段可以依赖这些不变量而无需重新接受裸模块数组。
pub struct CompilationUnit {
    pub(crate) limits: CompileProfile,
    pub(crate) modules: Box<[TypedAstModule]>,
    source_document_index: FrozenSourceDocumentIndex,
    pub(crate) source_document_count: u64,
    pub(crate) import_edge_count: u64,
    pub(crate) declaration_count: u64,
    pub(crate) stable_entity_count: u64,
    pub(crate) reference_count: u64,
    pub(crate) relation_occurrence_count: u64,
    pub(crate) identity_field_occurrence_count: u64,
    pub(crate) symbol_count: u64,
    pub(crate) maneuver_gate_count: u64,
    pub(crate) waiting_zone_count: u64,
    pub(crate) controlled_live_bytes: u64,
    pub(crate) admission_peak_live_bytes: u64,
}

impl CompilationUnit {
    /// 返回编译单元中的来源模块数。
    #[must_use]
    pub const fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// 返回独立登记的来源文档数。
    #[must_use]
    pub fn source_document_count(&self) -> usize {
        usize::try_from(self.source_document_count).unwrap_or(usize::MAX)
    }

    /// 按冻结后的依赖优先规范顺序遍历模块描述符。
    pub fn module_descriptors(&self) -> impl ExactSizeIterator<Item = &SourceModuleDescriptor> {
        self.modules.iter().map(|module| &module.descriptor)
    }

    /// 在模块规范顺序上，再按模块内文档键字节序遍历文档描述符。
    pub fn source_document_descriptors(&self) -> impl Iterator<Item = &SourceDocumentDescriptor> {
        self.modules
            .iter()
            .flat_map(|module| module.source_documents.iter())
    }

    /// 解析位置绑定的来源文档，并核对该文档属于预期规范模块。
    ///
    /// # Errors
    ///
    /// 文档键未登记或登记到另一逻辑模块时，返回结构化来源文档所有权诊断。
    #[inline]
    pub(crate) fn resolve_source_document_for_module(
        &self,
        owner_module_ordinal: u32,
        location: &SourceLocation,
    ) -> Result<SourceDocumentOrdinal, DiagnosticBundle> {
        let owner_module_index = usize::try_from(owner_module_ordinal)
            .expect("u32 module ordinals fit usize on supported targets");
        let expected_namespace = self.modules[owner_module_index]
            .descriptor()
            .authoring_namespace_id();
        let Some(binding) = self
            .source_document_index
            .resolve(location.source_document_key())
        else {
            return Err(DiagnosticBundle::single(
                Diagnostic::source_document_ownership_mismatch(
                    location.source_document_key(),
                    expected_namespace,
                    None,
                    location.clone(),
                ),
            ));
        };
        if binding.owner_module_ordinal() != owner_module_ordinal {
            let actual_module_index = usize::try_from(binding.owner_module_ordinal())
                .expect("u32 module ordinals fit usize on supported targets");
            let actual_namespace = self.modules[actual_module_index]
                .descriptor()
                .authoring_namespace_id();
            return Err(DiagnosticBundle::single(
                Diagnostic::source_document_ownership_mismatch(
                    location.source_document_key(),
                    expected_namespace,
                    Some(actual_namespace),
                    location.clone(),
                ),
            ));
        }
        Ok(binding.source_document_ordinal())
    }

    /// 把一个已经通过共同准入登记的位置解析成后续 IR 可直接携带的紧凑记录。
    #[inline]
    pub(crate) fn resolve_source_location_for_module(
        &self,
        owner_module_ordinal: u32,
        location: &SourceLocation,
    ) -> Result<ResolvedSourceLocation, DiagnosticBundle> {
        let source_document_ordinal =
            self.resolve_source_document_for_module(owner_module_ordinal, location)?;
        Ok(match location {
            SourceLocation::Text(span) => ResolvedSourceLocation::Text {
                source_document_ordinal,
                start: span.start(),
                end: span.end(),
            },
            SourceLocation::RoadEditing(location) => ResolvedSourceLocation::RoadEditing {
                source_document_ordinal,
                location: location.clone(),
            },
        })
    }

    /// 消费完整 Typed AST 输入，分别搬移源映射后续需要的模块与文档描述符。
    pub(crate) fn into_source_descriptors(
        self,
        pass: &crate::ledger::PassBudget<'_>,
    ) -> Result<
        (
            crate::ledger::Frozen<SourceModuleDescriptor>,
            crate::ledger::Frozen<SourceDocumentDescriptor>,
        ),
        crate::ledger::LedgerError,
    > {
        let mut source_modules = pass.vec(self.modules.len())?;
        let mut source_documents =
            pass.vec(usize::try_from(self.source_document_count).unwrap_or(usize::MAX))?;
        for module in self.modules.into_vec() {
            source_modules.push(module.descriptor)?;
            source_documents.try_extend(module.source_documents)?;
        }
        Ok((source_modules.freeze()?, source_documents.freeze()?))
    }
}

/// 按模块下标分段的环路列表：`ranges` 中每项是 `members` 内一条环路的起点与长度。
struct ModuleCycles {
    members: BoundedVec<usize>,
    ranges: BoundedVec<(usize, usize)>,
}

impl ModuleCycles {
    fn iter(&self) -> impl Iterator<Item = &[usize]> {
        self.ranges
            .iter()
            .map(|(start, len)| &self.members[*start..*start + *len])
    }
}

fn namespace_of(modules: &[AdmittedOfficialModule], index: usize) -> &Arc<str> {
    &modules[index].descriptor.authoring_namespace_id
}

/// 以 `less` 为序的最小堆；容量已由授予预留，压入不会扩容。
fn heap_push(
    heap: &mut BoundedVec<usize>,
    value: usize,
    less: impl Fn(usize, usize) -> bool,
) -> Result<(), LedgerError> {
    heap.push(value)?;
    let mut child = heap.len() - 1;
    while child > 0 {
        let parent = (child - 1) / 2;
        if !less(heap[child], heap[parent]) {
            break;
        }
        heap.swap(child, parent);
        child = parent;
    }
    Ok(())
}

fn heap_pop(heap: &mut BoundedVec<usize>, less: impl Fn(usize, usize) -> bool) -> Option<usize> {
    let last = heap.len().checked_sub(1)?;
    heap.swap(0, last);
    let top = heap.pop();
    let mut parent = 0;
    loop {
        let left = parent * 2 + 1;
        if left >= heap.len() {
            break;
        }
        let right = left + 1;
        let smaller = if right < heap.len() && less(heap[right], heap[left]) {
            right
        } else {
            left
        };
        if !less(heap[smaller], heap[parent]) {
            break;
        }
        heap.swap(parent, smaller);
        parent = smaller;
    }
    top
}

fn canonical_topological_order(
    pass: &PassBudget<'_>,
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Result<Result<BoundedVec<usize>, ModuleCycles>, LedgerError> {
    let module_count = modules.len();
    let mut indegree = pass.collect(modules.iter().map(|module| module.imports.len()))?;
    // 依赖方按依赖模块分段的连续表：`offsets[d]..offsets[d + 1]` 是依赖 `d` 的模块。
    let mut offsets = pass.collect(std::iter::repeat_n(0_usize, module_count + 1))?;
    for module in modules {
        for import in &module.imports {
            offsets[module_index[import.namespace.as_ref()] + 1] += 1;
        }
    }
    for index in 0..module_count {
        offsets[index + 1] += offsets[index];
    }
    let mut cursor = pass.collect(offsets[..module_count].iter().copied())?;
    let mut dependents = pass.collect(std::iter::repeat_n(0_usize, offsets[module_count]))?;
    for (index, module) in modules.iter().enumerate() {
        for import in &module.imports {
            let dependency = module_index[import.namespace.as_ref()];
            dependents[cursor[dependency]] = index;
            cursor[dependency] += 1;
        }
    }
    drop(cursor);
    for dependency in 0..module_count {
        dependents[offsets[dependency]..offsets[dependency + 1]].sort_unstable_by(|left, right| {
            namespace_of(modules, *left).cmp(namespace_of(modules, *right))
        });
    }

    // Kahn 就绪集按命名空间字节序出队：拓扑约束只规定依赖在前，命名空间唯一，为所有
    // 合法平局给出唯一顺序，避免模块加入顺序泄漏到规范输出。
    let less =
        |left: usize, right: usize| namespace_of(modules, left) < namespace_of(modules, right);
    let mut ready = pass.vec(module_count)?;
    for (index, degree) in indegree.iter().copied().enumerate() {
        if degree == 0 {
            heap_push(&mut ready, index, less)?;
        }
    }
    let mut order = pass.vec(module_count)?;
    while let Some(index) = heap_pop(&mut ready, less) {
        order.push(index)?;
        for dependent in dependents[offsets[index]..offsets[index + 1]]
            .iter()
            .copied()
        {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                heap_push(&mut ready, dependent, less)?;
            }
        }
    }

    if order.len() == module_count {
        Ok(Ok(order))
    } else {
        drop((indegree, offsets, dependents, ready, order));
        Ok(Err(find_canonical_cycles(pass, modules, module_index)?))
    }
}

fn sorted_dependencies(
    pass: &PassBudget<'_>,
    index: usize,
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Result<BoundedVec<usize>, LedgerError> {
    let mut dependencies = pass.collect(
        modules[index]
            .imports
            .iter()
            .map(|import| module_index[import.namespace.as_ref()]),
    )?;
    dependencies.sort_unstable_by(|left, right| {
        namespace_of(modules, *left).cmp(namespace_of(modules, *right))
    });
    Ok(dependencies)
}

fn find_canonical_cycles(
    pass: &PassBudget<'_>,
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Result<ModuleCycles, LedgerError> {
    // Tarjan 只负责找出强连通分量；遍历依赖、分量成员和最终分量列表都再按命名空间
    // 规范化，使诊断不依赖 HashMap 或来源导入顺序。
    struct Tarjan<'a, 'p> {
        pass: &'a PassBudget<'p>,
        modules: &'a [AdmittedOfficialModule],
        module_index: &'a HashMap<Arc<str>, usize>,
        next_index: usize,
        indices: BoundedVec<Option<usize>>,
        low_links: BoundedVec<usize>,
        stack: BoundedVec<usize>,
        on_stack: BoundedVec<bool>,
        members: BoundedVec<usize>,
        components: BoundedVec<(usize, usize)>,
    }

    impl Tarjan<'_, '_> {
        fn visit(&mut self, index: usize) -> Result<(), LedgerError> {
            let discovery_index = self.next_index;
            self.next_index += 1;
            self.indices[index] = Some(discovery_index);
            self.low_links[index] = discovery_index;
            self.stack.push(index)?;
            self.on_stack[index] = true;

            let dependencies =
                sorted_dependencies(self.pass, index, self.modules, self.module_index)?;
            for dependency in dependencies.iter().copied() {
                if self.indices[dependency].is_none() {
                    self.visit(dependency)?;
                    self.low_links[index] = self.low_links[index].min(self.low_links[dependency]);
                } else if self.on_stack[dependency]
                    && let Some(dependency_index) = self.indices[dependency]
                {
                    self.low_links[index] = self.low_links[index].min(dependency_index);
                }
            }
            drop(dependencies);

            if self.low_links[index] != discovery_index {
                return Ok(());
            }

            let start = self.members.len();
            while let Some(member) = self.stack.pop() {
                self.on_stack[member] = false;
                self.members.push(member)?;
                if member == index {
                    break;
                }
            }
            let modules = self.modules;
            self.members[start..].sort_unstable_by(|left, right| {
                namespace_of(modules, *left).cmp(namespace_of(modules, *right))
            });
            self.components.push((start, self.members.len() - start))?;
            Ok(())
        }
    }

    let module_count = modules.len();
    let mut canonical_indices = pass.collect(0..module_count)?;
    canonical_indices.sort_unstable_by(|left, right| {
        namespace_of(modules, *left).cmp(namespace_of(modules, *right))
    });
    let mut tarjan = Tarjan {
        pass,
        modules,
        module_index,
        next_index: 0,
        indices: pass.collect(std::iter::repeat_n(None, module_count))?,
        low_links: pass.collect(std::iter::repeat_n(0, module_count))?,
        stack: pass.vec(module_count)?,
        on_stack: pass.collect(std::iter::repeat_n(false, module_count))?,
        members: pass.vec(module_count)?,
        components: pass.vec(module_count)?,
    };
    for index in canonical_indices.iter().copied() {
        if tarjan.indices[index].is_none() {
            tarjan.visit(index)?;
        }
    }
    let Tarjan {
        members: component_members,
        components,
        ..
    } = tarjan;

    let mut cycles = ModuleCycles {
        members: pass.vec(module_count)?,
        ranges: pass.vec(module_count)?,
    };
    for (start, len) in components.iter().copied() {
        let component = &component_members[start..start + len];
        let cyclic = component.len() > 1
            || component.first().is_some_and(|index| {
                modules[*index]
                    .imports
                    .iter()
                    .any(|import| module_index[import.namespace.as_ref()] == *index)
            });
        if cyclic {
            let cycle_start = cycles.members.len();
            canonical_cycle_for_component(
                pass,
                component,
                modules,
                module_index,
                &mut cycles.members,
            )?;
            cycles
                .ranges
                .push((cycle_start, cycles.members.len() - cycle_start))?;
        }
    }
    let members = &cycles.members;
    cycles.ranges.sort_unstable_by(|left, right| {
        let left_namespace = (left.1 > 0).then(|| namespace_of(modules, members[left.0]));
        let right_namespace = (right.1 > 0).then(|| namespace_of(modules, members[right.0]));
        left_namespace.cmp(&right_namespace)
    });
    Ok(cycles)
}

/// 把一个强连通分量的规范环路见证追加到 `cycle`。
fn canonical_cycle_for_component(
    pass: &PassBudget<'_>,
    component: &[usize],
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
    cycle: &mut BoundedVec<usize>,
) -> Result<(), LedgerError> {
    // 一个 SCC 可能包含多条环。固定从字节序最小成员出发，并按规范依赖顺序寻找首条
    // 回路，为同一非法图选择稳定且可复现的诊断见证。`path` 是当前深度优先路径。
    #[allow(clippy::too_many_arguments)]
    fn find_path(
        pass: &PassBudget<'_>,
        index: usize,
        target: usize,
        allowed: &[bool],
        modules: &[AdmittedOfficialModule],
        module_index: &HashMap<Arc<str>, usize>,
        visited: &mut [bool],
        path: &mut BoundedVec<usize>,
    ) -> Result<bool, LedgerError> {
        if index == target {
            return Ok(true);
        }
        if visited[index] {
            return Ok(false);
        }
        visited[index] = true;
        path.push(index)?;
        for dependency in sorted_dependencies(pass, index, modules, module_index)?
            .iter()
            .copied()
        {
            if allowed[dependency]
                && find_path(
                    pass,
                    dependency,
                    target,
                    allowed,
                    modules,
                    module_index,
                    visited,
                    path,
                )?
            {
                return Ok(true);
            }
        }
        path.pop();
        Ok(false)
    }

    let Some(start) = component.first().copied() else {
        return Ok(());
    };
    let mut allowed = pass.collect(std::iter::repeat_n(false, modules.len()))?;
    for index in component.iter().copied() {
        allowed[index] = true;
    }
    for dependency in sorted_dependencies(pass, start, modules, module_index)?
        .iter()
        .copied()
    {
        if !allowed[dependency] {
            continue;
        }
        if dependency == start {
            return cycle.push(start);
        }
        let mut visited = pass.collect(std::iter::repeat_n(false, modules.len()))?;
        let mut path = pass.vec(modules.len())?;
        if find_path(
            pass,
            dependency,
            start,
            &allowed,
            modules,
            module_index,
            &mut visited,
            &mut path,
        )? {
            cycle.push(start)?;
            return cycle.extend_from_slice(&path);
        }
    }
    cycle.extend_from_slice(component)
}
