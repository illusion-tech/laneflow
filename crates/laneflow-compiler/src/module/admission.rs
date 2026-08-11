use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

#[cfg(test)]
use crate::SourceSpan;
use crate::arena::ArenaKey;
use crate::declaration::{RoadAlignmentDeclaration, TypedAstDeclaration};
use crate::diagnostic::DiagnosticCollector;
use crate::geometry_profile::GeometryCompilationProfiles;
use crate::{
    CompileLimitDimension, CompileLimits, Diagnostic, DiagnosticBundle, SourceLocation,
    SourcePosition,
};

use super::descriptor::{SourceDocumentDescriptor, SourceModuleDescriptor};
#[cfg(test)]
use super::descriptor::{SourceDocumentOrigin, freeze_source_documents, source_document_digest};
use super::resources::size_bytes;
use super::resources::{AdmissionTotals, ModuleResourceCounts, requested_hash_table_bytes};
use super::synthetic::SyntheticModule;

/// 区分编译单元来源文档登记序号的零尺寸标记。
pub(crate) enum SourceDocumentTag {}
/// 仅在同一次编译的来源模块描述符表内有效的致密序号。
pub(crate) type SourceDocumentOrdinal = ArenaKey<SourceDocumentTag>;

pub(super) struct ImportRecord {
    pub(super) namespace: Arc<str>,
    pub(super) span: SourceLocation,
}

/// 官方前端完成受检构造后交给共同编译管线的 Typed AST 模块。
pub(crate) struct TypedAstModule {
    pub(crate) descriptor: SourceModuleDescriptor,
    pub(crate) declaration_span: SourceLocation,
    pub(crate) source_documents: Box<[SourceDocumentDescriptor]>,
    pub(super) imports: Box<[ImportRecord]>,
    /// 只对已经执行 authoring numeric freeze 的官方来源存在；共同 HIR 在完整模块图上
    /// 校验所有此类模块使用同一对位置/方向档。
    pub(crate) geometry_profiles: Option<GeometryCompilationProfiles>,
    pub(crate) road_alignments: Box<[RoadAlignmentDeclaration]>,
    pub(crate) declarations: Box<[TypedAstDeclaration]>,
}

impl TypedAstModule {
    pub(crate) const fn descriptor(&self) -> &SourceModuleDescriptor {
        &self.descriptor
    }

    pub(crate) const fn declaration_span(&self) -> &SourceLocation {
        &self.declaration_span
    }

    pub(crate) fn import_records(&self) -> impl ExactSizeIterator<Item = (&str, &SourceLocation)> {
        self.imports
            .iter()
            .map(|record| (record.namespace.as_ref(), &record.span))
    }
}

/// 描述符、文档、Typed AST 与准入资源计数不可分的内部载荷。
pub(super) struct AdmittedOfficialModule {
    typed_ast: TypedAstModule,
    pub(super) resource_counts: ModuleResourceCounts,
}

impl AdmittedOfficialModule {
    pub(super) fn new(typed_ast: TypedAstModule, resource_counts: ModuleResourceCounts) -> Self {
        assert!(
            typed_ast.source_documents.windows(2).all(|pair| {
                pair[0].source_document_key.as_bytes() <= pair[1].source_document_key.as_bytes()
            }),
            "official frontends must canonically sort source documents before common admission"
        );
        Self {
            typed_ast,
            resource_counts,
        }
    }

    pub(super) const fn typed_ast(&self) -> &TypedAstModule {
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

#[cfg(test)]
pub(super) struct TestSourceDocument<'a> {
    pub(super) source_document_key: &'a str,
    pub(super) source_record: &'a [u8],
    pub(super) display_source: Option<&'a str>,
}

#[cfg(test)]
impl TestOfficialModule {
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
        let AdmittedOfficialModule {
            typed_ast,
            resource_counts,
        } = admitted;
        Self {
            admitted: AdmittedOfficialModule::new(typed_ast, resource_counts),
        }
    }

    pub(super) fn move_first_lane_edge_span_to(&mut self, source_document_key: &str) {
        let TypedAstDeclaration::LaneEdge(declaration) =
            &mut self.admitted.typed_ast.declarations[0]
        else {
            panic!("test wrapper expected first declaration to be LaneEdge");
        };
        declaration.header.span = SourceSpan::point(Arc::from(source_document_key), 41, 7).into();
    }

    pub(super) fn move_module_declaration_span_to(&mut self, source_document_key: &str) {
        self.admitted.typed_ast.declaration_span =
            SourceSpan::point(Arc::from(source_document_key), 37, 5).into();
    }

    pub(super) fn from_synthetic_with_unsorted_documents(
        module: SyntheticModule,
        documents: &[(&str, &[u8])],
    ) -> Self {
        let mut module = Self::from_synthetic_with_documents(module, documents);
        module.admitted.typed_ast.source_documents.swap(1, 2);
        let AdmittedOfficialModule {
            typed_ast,
            resource_counts,
        } = module.admitted;
        Self {
            admitted: AdmittedOfficialModule::new(typed_ast, resource_counts),
        }
    }

    pub(super) fn move_first_lane_edge_successor_span_to(&mut self, source_document_key: &str) {
        let TypedAstDeclaration::LaneEdge(declaration) =
            &mut self.admitted.typed_ast.declarations[0]
        else {
            panic!("test wrapper expected first declaration to be LaneEdge");
        };
        declaration.successors[0].span =
            SourceSpan::point(Arc::from(source_document_key), 43, 9).into();
    }

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
                    if let Some(reference) = &mut space.parking_area {
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

    pub(super) fn force_resource_count(&mut self, dimension: CompileLimitDimension, observed: u64) {
        let counts = &mut self.admitted.resource_counts;
        match dimension {
            CompileLimitDimension::DeclarationCount => counts.declaration_count = observed,
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
            CompileLimitDimension::RouteOccurrenceCount => counts.route_occurrence_count = observed,
            CompileLimitDimension::GeometryPointCount => counts.geometry_point_count = observed,
            _ => panic!("test dimension is not represented by ModuleResourceCounts"),
        }
    }
}

const UNBOUND_SOURCE_DOCUMENT_ORDINAL: u32 = u32::MAX;

#[derive(Clone, Copy)]
struct SourceDocumentBinding {
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
    bindings: HashMap<Arc<str>, SourceDocumentBinding>,
}

impl PendingSourceDocumentIndex {
    fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    #[inline]
    fn get(&self, source_document_key: &str) -> Option<SourceDocumentBinding> {
        self.bindings.get(source_document_key).copied()
    }

    #[inline]
    fn insert(
        &mut self,
        source_document_key: Arc<str>,
        binding: SourceDocumentBinding,
    ) -> Option<SourceDocumentBinding> {
        self.bindings.insert(source_document_key, binding)
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    fn freeze(
        mut self,
        modules: &[(usize, AdmittedOfficialModule)],
        expected_document_count: u64,
    ) -> FrozenSourceDocumentIndex {
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
        FrozenSourceDocumentIndex {
            bindings: self.bindings,
        }
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
    pub(crate) const fn owner_module_ordinal(self) -> u32 {
        self.owner_module_ordinal
    }

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

#[inline]
pub(super) fn source_document_index_requested_bytes(source_document_count: u64) -> u64 {
    requested_hash_table_bytes::<Arc<str>, SourceDocumentBinding>(source_document_count)
}

#[inline]
fn module_index_requested_bytes(module_count: u64) -> u64 {
    requested_hash_table_bytes::<Arc<str>, usize>(module_count)
}

#[inline]
fn ordered_ready_set_requested_bytes(module_count: u64) -> u64 {
    // `BTreeSet` 没有稳定公开的节点布局。这里沿用共同哈希索引的八桶保守预算，
    // 只把它当作有界有序就绪集的保守请求字节预算，不依赖标准库私有实现。
    requested_hash_table_bytes::<(Arc<str>, usize), ()>(module_count)
}

#[inline]
fn builder_live_requested_bytes(totals: AdmissionTotals) -> u64 {
    totals
        .module_payload_live_bytes
        .saturating_add(source_document_index_requested_bytes(
            totals.source_document_count,
        ))
        .saturating_add(module_index_requested_bytes(totals.module_count))
        .saturating_add(size_bytes::<AdmittedOfficialModule>(totals.module_count))
}

/// 共同准入的三层内存账本：构建器存续量、冻结阶段暂存峰值与成功结果存续量。
///
/// 所有固定工作集项都只采用输入计数、可审计类型布局和显式保守索引模型计算，因而可以
/// 在相应分配发生前完成限额检查。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct AdmissionSizing {
    pub(super) builder_live_bytes: u64,
    pub(super) result_live_bytes: u64,
    pub(super) build_scratch_bytes: u64,
    pub(super) build_peak_live_bytes: u64,
}

impl AdmissionSizing {
    pub(super) fn from_totals(totals: AdmissionTotals, diagnostic_limit: u64) -> Self {
        let module_count = totals.module_count;
        let import_edge_count = totals.import_edge_count;
        let module_payload_live_bytes = totals.module_payload_live_bytes;
        let source_document_index_bytes =
            source_document_index_requested_bytes(totals.source_document_count);
        let module_index_bytes = module_index_requested_bytes(module_count);
        let typed_ast_module_bytes = size_bytes::<TypedAstModule>(module_count);
        let reordered_module_bytes = size_bytes::<(usize, AdmittedOfficialModule)>(module_count);

        let canonical_index_bytes = size_bytes::<usize>(module_count);
        // `DiagnosticCollector` 一开始就按配置档上限申请缓冲区，并存续到 `build` 返回。
        let diagnostic_buffer_bytes = size_bytes::<Diagnostic>(diagnostic_limit);
        let topology_scratch_bytes = size_bytes::<usize>(module_count)
            .saturating_add(size_bytes::<Vec<usize>>(module_count))
            .saturating_add(size_bytes::<usize>(import_edge_count))
            .saturating_add(ordered_ready_set_requested_bytes(module_count))
            .saturating_add(size_bytes::<usize>(module_count));
        // 环路路径递归期间，每层的规范依赖向量可能同时存续；所有依赖项总数提供上界。
        // 返回路径搬移期间至多有完整路径及其后缀两份 `usize` 缓冲区共存。
        let tarjan_scratch_bytes = size_bytes::<usize>(module_count)
            .saturating_add(size_bytes::<Option<usize>>(module_count))
            .saturating_add(size_bytes::<usize>(module_count))
            .saturating_add(size_bytes::<usize>(module_count))
            .saturating_add(size_bytes::<bool>(module_count))
            .saturating_add(size_bytes::<Vec<usize>>(module_count))
            .saturating_add(size_bytes::<usize>(module_count))
            .saturating_add(size_bytes::<usize>(import_edge_count.saturating_mul(2)))
            .saturating_add(size_bytes::<bool>(module_count.saturating_mul(2)))
            .saturating_add(size_bytes::<usize>(module_count.saturating_mul(2)))
            .saturating_add(size_bytes::<Vec<usize>>(module_count))
            .saturating_add(size_bytes::<usize>(module_count));
        let graph_scratch_bytes = diagnostic_buffer_bytes
            .saturating_add(canonical_index_bytes)
            .saturating_add(topology_scratch_bytes)
            .saturating_add(tarjan_scratch_bytes);

        let cycle_diagnostic_scratch_bytes = diagnostic_buffer_bytes
            .saturating_add(canonical_index_bytes)
            .saturating_add(size_bytes::<Vec<usize>>(module_count))
            .saturating_add(size_bytes::<usize>(module_count))
            .saturating_add(size_bytes::<u32>(module_count))
            .saturating_add(size_bytes::<&str>(module_count))
            .saturating_add(size_bytes::<SourceLocation>(module_count));

        let rank_bytes = size_bytes::<usize>(module_count);
        let rank_scratch_bytes = diagnostic_buffer_bytes
            .saturating_add(canonical_index_bytes)
            .saturating_add(size_bytes::<usize>(module_count))
            .saturating_add(rank_bytes);
        let reorder_scratch_bytes = diagnostic_buffer_bytes
            .saturating_add(canonical_index_bytes)
            .saturating_add(rank_bytes)
            .saturating_add(reordered_module_bytes);
        let build_scratch_bytes = graph_scratch_bytes
            .max(cycle_diagnostic_scratch_bytes)
            .max(rank_scratch_bytes)
            .max(reorder_scratch_bytes);

        let builder_live_bytes = builder_live_requested_bytes(totals);
        let result_live_bytes = module_payload_live_bytes
            .saturating_add(source_document_index_bytes)
            .saturating_add(typed_ast_module_bytes);
        let graph_peak_live_bytes = builder_live_bytes.saturating_add(graph_scratch_bytes);
        // `collect` 期间旧模块向量和重排元组向量会短暂共存。
        let reorder_peak_live_bytes = builder_live_bytes
            .saturating_add(diagnostic_buffer_bytes)
            .saturating_add(canonical_index_bytes)
            .saturating_add(rank_bytes)
            .saturating_add(reordered_module_bytes);
        // 元组向量被消费为最终 Typed AST 向量时，模块索引直到 `build` 返回才释放。
        let conversion_peak_live_bytes = module_payload_live_bytes
            .saturating_add(source_document_index_bytes)
            .saturating_add(module_index_bytes)
            .saturating_add(reordered_module_bytes)
            .saturating_add(typed_ast_module_bytes)
            .saturating_add(diagnostic_buffer_bytes)
            .saturating_add(canonical_index_bytes)
            .saturating_add(rank_bytes);
        let build_peak_live_bytes = graph_peak_live_bytes
            .max(reorder_peak_live_bytes)
            .max(conversion_peak_live_bytes)
            .max(result_live_bytes);

        Self {
            builder_live_bytes,
            result_live_bytes,
            build_scratch_bytes,
            build_peak_live_bytes,
        }
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
/// use laneflow_compiler::{CompilationUnitBuilder, CompileLimits};
/// let mut builder = CompilationUnitBuilder::new(CompileLimits::p100_initial_v2());
/// let _generic_admission = builder.add_module;
/// ```
pub struct CompilationUnitBuilder {
    limits: CompileLimits,
    pub(super) modules: Vec<AdmittedOfficialModule>,
    pub(super) module_index: HashMap<Arc<str>, usize>,
    source_document_index: PendingSourceDocumentIndex,
    pub(super) totals: AdmissionTotals,
}

/// 已完整校验、尚未写入构建器的单模块准入事务。
struct AdmissionPlan {
    module: AdmittedOfficialModule,
    next_totals: AdmissionTotals,
}

impl CompilationUnitBuilder {
    /// 用同一份显式资源配置档建立空编译单元构建器。
    #[must_use]
    pub fn new(limits: CompileLimits) -> Self {
        Self {
            limits,
            modules: Vec::new(),
            module_index: HashMap::new(),
            source_document_index: PendingSourceDocumentIndex::new(),
            totals: AdmissionTotals::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn source_document_index_is_empty(&self) -> bool {
        self.source_document_index.is_empty()
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

    #[cfg(test)]
    pub(super) fn add_test_official_module(
        &mut self,
        module: TestOfficialModule,
    ) -> Result<&mut Self, DiagnosticBundle> {
        self.admit_official_module(module.admitted)
    }

    #[inline]
    fn admit_official_module(
        &mut self,
        module: AdmittedOfficialModule,
    ) -> Result<&mut Self, DiagnosticBundle> {
        let plan = self.prepare_admission(module)?;
        self.commit_admission(plan);
        Ok(self)
    }

    #[inline]
    fn prepare_admission(
        &self,
        module: AdmittedOfficialModule,
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
        if document_count > 1 && self.limits.source_document_count_limit().is_none() {
            return Err(DiagnosticBundle::single(
                Diagnostic::compile_profile_incompatible(
                    self.limits.profile_id(),
                    CompileLimitDimension::SourceDocumentCount,
                    module.declaration_span.clone(),
                    namespace,
                ),
            ));
        }
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

        let next_totals = self.totals.candidate_after(
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
        let next_builder_live_bytes = builder_live_requested_bytes(next_totals);
        for (dimension, observed) in next_totals.limit_observations(next_builder_live_bytes) {
            let limit = self.limits.value(dimension);
            if observed > limit {
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
        }
        if let Some(limit) = self.limits.source_document_count_limit()
            && next_totals.source_document_count > limit
        {
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

        Ok(AdmissionPlan {
            module,
            next_totals,
        })
    }

    #[inline]
    fn commit_admission(&mut self, plan: AdmissionPlan) {
        let namespace = Arc::clone(&plan.module.descriptor.authoring_namespace_id);
        let module_index = self.modules.len();
        self.modules.push(plan.module);
        self.module_index.insert(namespace, module_index);
        for document in &self.modules[module_index].source_documents {
            let previous = self.source_document_index.insert(
                document.source_document_key_arc(),
                SourceDocumentBinding::pending(module_index),
            );
            debug_assert!(
                previous.is_none(),
                "admission plan guaranteed unique document keys"
            );
        }
        self.totals = plan.next_totals;
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
    pub fn build(self) -> Result<CompilationUnit, DiagnosticBundle> {
        let sizing = AdmissionSizing::from_totals(
            self.totals,
            self.limits.value(CompileLimitDimension::DiagnosticCount),
        );
        let primary_module = self.modules.iter().min_by(|left, right| {
            left.descriptor
                .authoring_namespace_id
                .cmp(&right.descriptor.authoring_namespace_id)
        });
        for (dimension, observed) in [
            (
                CompileLimitDimension::StageScratchBytes,
                sizing.build_scratch_bytes,
            ),
            (
                CompileLimitDimension::CompilerControlledLiveBytes,
                sizing.build_peak_live_bytes,
            ),
        ] {
            let limit = self.limits.value(dimension);
            if observed > limit {
                return Err(DiagnosticBundle::single(
                    Diagnostic::compile_limit_exceeded_at(
                        dimension,
                        limit,
                        observed,
                        primary_module.map(|module| module.declaration_span.clone()),
                        primary_module
                            .map(|module| module.descriptor.authoring_namespace_id.as_ref().into()),
                    ),
                ));
            }
        }

        let mut diagnostics =
            DiagnosticCollector::new(self.limits.value(CompileLimitDimension::DiagnosticCount));
        let mut canonical_indices: Vec<_> = (0..self.modules.len()).collect();
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

        let order = match canonical_topological_order(&self.modules, &self.module_index) {
            Ok(order) => order,
            Err(cycles) => {
                let mut canonical_order_by_index = vec![0_u32; self.modules.len()];
                for (order, module_index) in canonical_indices.iter().copied().enumerate() {
                    canonical_order_by_index[module_index] =
                        u32::try_from(order).unwrap_or(u32::MAX);
                }
                for cycle in cycles {
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

        let mut canonical_rank = vec![0_usize; order.len()];
        for (rank, index) in order.into_iter().enumerate() {
            canonical_rank[index] = rank;
        }
        let mut modules: Vec<_> = self.modules.into_iter().enumerate().collect();
        modules.sort_unstable_by_key(|(original_index, _)| canonical_rank[*original_index]);
        let source_document_index = self
            .source_document_index
            .freeze(&modules, self.totals.source_document_count);
        let modules = modules
            .into_iter()
            .map(|(_, module)| module.typed_ast)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(CompilationUnit {
            limits: self.limits,
            modules,
            source_document_index,
            source_document_count: self.totals.source_document_count,
            import_edge_count: self.totals.import_edge_count,
            declaration_count: self.totals.declaration_count,
            reference_count: self.totals.reference_count,
            relation_occurrence_count: self.totals.relation_occurrence_count,
            identity_field_occurrence_count: self.totals.identity_field_occurrence_count,
            symbol_count: self.totals.symbol_count,
            maneuver_gate_count: self.totals.maneuver_gate_count,
            waiting_zone_count: self.totals.waiting_zone_count,
            route_occurrence_count: self.totals.route_occurrence_count,
            controlled_live_bytes: sizing.result_live_bytes,
        })
    }
}

/// 规范模块顺序已冻结的原子编译输入。
///
/// 构造完成后，全部导入目标存在、导入图无环，并且 `modules` 按依赖优先的规范顺序
/// 排列。类型字段私有，后续阶段可以依赖这些不变量而无需重新接受裸模块数组。
pub struct CompilationUnit {
    pub(crate) limits: CompileLimits,
    pub(crate) modules: Box<[TypedAstModule]>,
    source_document_index: FrozenSourceDocumentIndex,
    pub(crate) source_document_count: u64,
    pub(crate) import_edge_count: u64,
    pub(crate) declaration_count: u64,
    pub(crate) reference_count: u64,
    pub(crate) relation_occurrence_count: u64,
    pub(crate) identity_field_occurrence_count: u64,
    pub(crate) symbol_count: u64,
    pub(crate) maneuver_gate_count: u64,
    pub(crate) waiting_zone_count: u64,
    pub(crate) route_occurrence_count: u64,
    pub(crate) controlled_live_bytes: u64,
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
    ) -> (
        Box<[SourceModuleDescriptor]>,
        Box<[SourceDocumentDescriptor]>,
    ) {
        let mut source_modules = Vec::with_capacity(self.modules.len());
        let mut source_documents =
            Vec::with_capacity(usize::try_from(self.source_document_count).unwrap_or(usize::MAX));
        for module in self.modules.into_vec() {
            source_modules.push(module.descriptor);
            source_documents.extend(module.source_documents);
        }
        (
            source_modules.into_boxed_slice(),
            source_documents.into_boxed_slice(),
        )
    }
}

fn canonical_topological_order(
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Result<Vec<usize>, Vec<Vec<usize>>> {
    let mut indegree = vec![0_usize; modules.len()];
    let mut dependents = vec![Vec::new(); modules.len()];
    for (index, module) in modules.iter().enumerate() {
        indegree[index] = module.imports.len();
        for import in &module.imports {
            let dependency_index = module_index[import.namespace.as_ref()];
            dependents[dependency_index].push(index);
        }
    }
    for entries in &mut dependents {
        entries.sort_unstable_by(|left, right| {
            modules[*left]
                .descriptor
                .authoring_namespace_id
                .cmp(&modules[*right].descriptor.authoring_namespace_id)
        });
    }

    // Kahn 就绪集同时携带命名空间与原索引：拓扑约束只规定依赖在前，BTreeSet 为所有
    // 合法平局给出唯一字节序，避免模块加入顺序泄漏到规范输出。
    let mut ready = BTreeSet::new();
    for (index, degree) in indegree.iter().copied().enumerate() {
        if degree == 0 {
            ready.insert((
                Arc::clone(&modules[index].descriptor.authoring_namespace_id),
                index,
            ));
        }
    }
    let mut order = Vec::with_capacity(modules.len());
    while let Some((_, index)) = ready.pop_first() {
        order.push(index);
        for dependent in dependents[index].iter().copied() {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                ready.insert((
                    Arc::clone(&modules[dependent].descriptor.authoring_namespace_id),
                    dependent,
                ));
            }
        }
    }

    if order.len() == modules.len() {
        Ok(order)
    } else {
        Err(find_canonical_cycles(modules, module_index))
    }
}

fn sorted_dependencies(
    index: usize,
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Vec<usize> {
    let mut dependencies: Vec<_> = modules[index]
        .imports
        .iter()
        .map(|import| module_index[import.namespace.as_ref()])
        .collect();
    dependencies.sort_unstable_by(|left, right| {
        modules[*left]
            .descriptor
            .authoring_namespace_id
            .cmp(&modules[*right].descriptor.authoring_namespace_id)
    });
    dependencies
}

fn find_canonical_cycles(
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Vec<Vec<usize>> {
    // Tarjan 只负责找出强连通分量；遍历依赖、分量成员和最终分量列表都再按命名空间
    // 规范化，使诊断不依赖 HashMap 或来源导入顺序。
    struct Tarjan<'a> {
        modules: &'a [AdmittedOfficialModule],
        module_index: &'a HashMap<Arc<str>, usize>,
        next_index: usize,
        indices: Vec<Option<usize>>,
        low_links: Vec<usize>,
        stack: Vec<usize>,
        on_stack: Vec<bool>,
        components: Vec<Vec<usize>>,
    }

    impl Tarjan<'_> {
        fn visit(&mut self, index: usize) {
            let discovery_index = self.next_index;
            self.next_index += 1;
            self.indices[index] = Some(discovery_index);
            self.low_links[index] = discovery_index;
            self.stack.push(index);
            self.on_stack[index] = true;

            for dependency in sorted_dependencies(index, self.modules, self.module_index) {
                if self.indices[dependency].is_none() {
                    self.visit(dependency);
                    self.low_links[index] = self.low_links[index].min(self.low_links[dependency]);
                } else if self.on_stack[dependency]
                    && let Some(dependency_index) = self.indices[dependency]
                {
                    self.low_links[index] = self.low_links[index].min(dependency_index);
                }
            }

            if self.low_links[index] != discovery_index {
                return;
            }

            let mut component = Vec::new();
            while let Some(member) = self.stack.pop() {
                self.on_stack[member] = false;
                component.push(member);
                if member == index {
                    break;
                }
            }
            component.sort_unstable_by(|left, right| {
                self.modules[*left]
                    .descriptor
                    .authoring_namespace_id
                    .cmp(&self.modules[*right].descriptor.authoring_namespace_id)
            });
            self.components.push(component);
        }
    }

    let mut canonical_indices: Vec<_> = (0..modules.len()).collect();
    canonical_indices.sort_unstable_by(|left, right| {
        modules[*left]
            .descriptor
            .authoring_namespace_id
            .cmp(&modules[*right].descriptor.authoring_namespace_id)
    });
    let mut tarjan = Tarjan {
        modules,
        module_index,
        next_index: 0,
        indices: vec![None; modules.len()],
        low_links: vec![0; modules.len()],
        stack: Vec::new(),
        on_stack: vec![false; modules.len()],
        components: Vec::new(),
    };
    for index in canonical_indices {
        if tarjan.indices[index].is_none() {
            tarjan.visit(index);
        }
    }

    let mut cycles: Vec<_> = tarjan
        .components
        .into_iter()
        .filter(|component| {
            component.len() > 1
                || component.first().is_some_and(|index| {
                    modules[*index]
                        .imports
                        .iter()
                        .any(|import| module_index[import.namespace.as_ref()] == *index)
                })
        })
        .map(|component| canonical_cycle_for_component(&component, modules, module_index))
        .collect();
    cycles.sort_unstable_by(|left, right| {
        let left_namespace = left
            .first()
            .map(|index| &modules[*index].descriptor.authoring_namespace_id);
        let right_namespace = right
            .first()
            .map(|index| &modules[*index].descriptor.authoring_namespace_id);
        left_namespace.cmp(&right_namespace)
    });
    cycles
}

fn canonical_cycle_for_component(
    component: &[usize],
    modules: &[AdmittedOfficialModule],
    module_index: &HashMap<Arc<str>, usize>,
) -> Vec<usize> {
    // 一个 SCC 可能包含多条环。固定从字节序最小成员出发，并按规范依赖顺序寻找首条
    // 回路，为同一非法图选择稳定且可复现的诊断见证。
    fn find_path(
        index: usize,
        target: usize,
        allowed: &[bool],
        modules: &[AdmittedOfficialModule],
        module_index: &HashMap<Arc<str>, usize>,
        visited: &mut [bool],
    ) -> Option<Vec<usize>> {
        if index == target {
            return Some(vec![target]);
        }
        if visited[index] {
            return None;
        }
        visited[index] = true;
        for dependency in sorted_dependencies(index, modules, module_index) {
            if allowed[dependency]
                && let Some(mut suffix) =
                    find_path(dependency, target, allowed, modules, module_index, visited)
            {
                let mut path = Vec::with_capacity(suffix.len() + 1);
                path.push(index);
                path.append(&mut suffix);
                return Some(path);
            }
        }
        None
    }

    let Some(start) = component.first().copied() else {
        return Vec::new();
    };
    let mut allowed = vec![false; modules.len()];
    for index in component.iter().copied() {
        allowed[index] = true;
    }
    for dependency in sorted_dependencies(start, modules, module_index) {
        if !allowed[dependency] {
            continue;
        }
        if dependency == start {
            return vec![start];
        }
        let mut visited = vec![false; modules.len()];
        if let Some(mut path) = find_path(
            dependency,
            start,
            &allowed,
            modules,
            module_index,
            &mut visited,
        ) {
            path.pop();
            let mut cycle = Vec::with_capacity(path.len() + 1);
            cycle.push(start);
            cycle.append(&mut path);
            return cycle;
        }
    }
    component.to_vec()
}
